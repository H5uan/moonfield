//! The per-view splat forward chain: the `gs/forward.slang` kernels recorded
//! into the `Core3d` schedule after the opaque pass.
//!
//! Per view per frame, [`splat_forward`] records, through the
//! [`RenderContext`] compute door (whose automatic `COMPUTE → COMPUTE`
//! barriers serialize every link):
//!
//! `clear` → per cloud `project_cull` + `duplicate_keys` → depth `RadixSort`
//! over the full entry capacity → `gather` → tile `RadixSort` →
//! `clear_tile_ranges` → `tile_ranges` → `blend`.
//!
//! The appended entry count never leaves the GPU: `clear` sentinel-fills the
//! tile keys, both sorts run over the full capacity (the sentinels cluster at
//! the end), and `tile_ranges` skips them. Multiple clouds share one entry
//! array — each cloud's `duplicate_keys` writes global record ids via its
//! `base` — so one sort+blend covers the view.
//!
//! Persistent state lives in render-world resources:
//! [`SplatForwardPipelines`] (built in `PrepareViews` from the prepared
//! `gaussian.slang` asset, revision-tracked like `SplatSortPass`) and
//! [`SplatViewArtifacts`] (per-view buffers, the RGBA16F splat image, and the
//! `RadixSort`, rebuilt when the view's extent or total Gaussian count
//! changes).

use std::collections::HashMap;

use moonfield_app::prelude::{App, IntoSystemConfigs, Plugin, World};
use moonfield_app::{ExtractSchedule, Render};
use moonfield_asset::{AssetRevision, Handle};
use moonfield_log::{error, error_once};
use moonfield_math::{Mat3, Vec3};
use moonfield_render_core::schedule as render_sets;
use moonfield_render_core::{
    ComputeRecording, CurrentView, ExtractedView, MainEntity, RenderContext, ViewAttachments,
    extract_with_transform,
};
use moonfield_rhi::{
    ComputePipeline, Device, Format, GpuAllocation, GpuPtr, MemoryResidency, RenderDevice, Result,
    RootBinder, RootParamPlace, ShaderModule, Texture,
};
use moonfield_shader::Shader;

use crate::core_3d::Core3d;
use crate::core_3d::pass::opaque_pass_3d;
use crate::gpu_util::RadixSort;
use crate::shader::{
    PipelineShader, PipelineShaders, PreparedShader, PreparedShaders, ShaderEntry,
};
use crate::splat::cloud::SplatCloudHandle;
use crate::splat::forward::{
    BlendParams, ClearParams, ClearTileRangesParams, DuplicateKeysParams, GatherParams,
    ProjectCullParams, SplatRecord, SplatView, TileRangesParams,
};
use crate::splat::prepare::{PreparedSplats, extract_splat_assets, prepare_splats};
use crate::splat::sort_pass::SPLAT_SORT_SHADER;

/// The name keying the splat forward chain's shader in [`PipelineShaders`]
/// and [`PreparedShaders`].
pub const SPLAT_FORWARD_SHADER: &str = "splat_forward";

/// The `gs/forward.slang` tile edge length in pixels.
const TILE_SIZE: u32 = 16;

/// Entry capacity per Gaussian: the tile-rectangle budget (16 tiles per
/// splat on average). Overflow past it is dropped and counted in the
/// counters buffer — observed, not fatal.
const ENTRIES_PER_GAUSSIAN: u32 = 16;

/// The forward chain's entries. `blend` writes the splat image through the
/// descriptor heap, so it compiles with the backend's heap capability; the
/// rest are plain pointer kernels.
const SPLAT_FORWARD_ENTRIES: &[ShaderEntry] = &[
    ShaderEntry {
        name: "clear",
        capabilities: &[],
    },
    ShaderEntry {
        name: "project_cull",
        capabilities: &[],
    },
    ShaderEntry {
        name: "duplicate_keys",
        capabilities: &[],
    },
    ShaderEntry {
        name: "gather",
        capabilities: &[],
    },
    ShaderEntry {
        name: "clear_tile_ranges",
        capabilities: &[],
    },
    ShaderEntry {
        name: "tile_ranges",
        capabilities: &[],
    },
    ShaderEntry {
        name: "blend",
        capabilities: moonfield_rhi::shader_capabilities(),
    },
];

/// The splat forward chain's shader request: `gaussian.slang`, seven compute
/// entries, root binding reflected from `project_cull` (the prepared
/// reflection covers every declared entry). The caller (the app wiring, same
/// as for the mesh pipeline's shaders) supplies the loaded asset handle.
pub fn splat_forward_shader(shader: Handle<Shader>) -> PipelineShader {
    PipelineShader {
        pipeline: SPLAT_FORWARD_SHADER,
        shader,
        reflect_entry: "project_cull",
        entries: SPLAT_FORWARD_ENTRIES,
    }
}

/// One entry's pipeline plus its `params` root-blob placement.
struct ForwardEntry {
    pipeline: ComputePipeline,
    place: RootParamPlace,
}

/// The forward chain's seven compute pipelines, built from the prepared
/// `gaussian.slang` asset and rebuilt when its revision advances.
pub struct SplatForwardPipelines {
    clear: ForwardEntry,
    project_cull: ForwardEntry,
    duplicate_keys: ForwardEntry,
    gather: ForwardEntry,
    clear_tile_ranges: ForwardEntry,
    tile_ranges: ForwardEntry,
    blend: ForwardEntry,
    /// The shader asset the pipelines were built from.
    shader: Handle<Shader>,
    /// The prepared-shader revision the pipelines were built from.
    shader_revision: AssetRevision,
}

impl SplatForwardPipelines {
    /// Build every pipeline from one prepared shader (the artifacts
    /// [`PreparedShaders`] compiled from the extracted asset).
    fn from_prepared(
        device: &Device,
        shader: Handle<Shader>,
        prepared: &PreparedShader,
    ) -> Result<Self> {
        let build = |entry: &str| -> Result<ForwardEntry> {
            let binder = RootBinder::new(prepared.reflection(), entry)?;
            // Each entry takes a single `uniform Params` struct; reflect its
            // one root placement and push it as one POD blob per dispatch.
            let place = binder.uniform_param("params")?;
            let artifact = prepared.entry(entry).ok_or_else(|| {
                moonfield_rhi::Error::Backend(format!(
                    "prepared splat forward shader is missing '{entry}'"
                ))
            })?;
            let module = ShaderModule::from_compiled(device, artifact)?;
            let pipeline = ComputePipeline::new(device, &module)?;
            Ok(ForwardEntry { pipeline, place })
        };
        Ok(Self {
            clear: build("clear")?,
            project_cull: build("project_cull")?,
            duplicate_keys: build("duplicate_keys")?,
            gather: build("gather")?,
            clear_tile_ranges: build("clear_tile_ranges")?,
            tile_ranges: build("tile_ranges")?,
            blend: build("blend")?,
            shader,
            shader_revision: prepared.revision(),
        })
    }
}

/// One view's splat forward buffers: the RGBA16F splat image, the
/// transmittance artifact, the concatenated `SplatRecord` array, the entry
/// buffers the sorts permute, and the per-frame camera record.
pub struct ViewArtifacts {
    extent: (u32, u32),
    gaussian_count: u32,
    capacity: u32,
    /// The radix sort both orderings run through; its `max_count` is the
    /// entry capacity. One instance serves both sorts — the recording's
    /// automatic barriers serialize them.
    sort: RadixSort,
    /// The sort shader identity the sort was built from.
    sort_shader: Handle<Shader>,
    sort_revision: AssetRevision,
    image: Texture,
    transmittance: GpuAllocation,
    records: GpuAllocation,
    counters: GpuAllocation,
    /// duplicate's (tile, depth, id) triples at atomic-append slots.
    tile_keys: GpuAllocation,
    depth_keys: GpuAllocation,
    ids: GpuAllocation,
    /// Constant 0..capacity; the depth sort's value input, filled once here.
    slot_identity: GpuAllocation,
    /// The depth sort's output: sorted depth keys + original slot indices.
    depth_sorted: GpuAllocation,
    slots_sorted: GpuAllocation,
    /// The gather's output: tile keys and ids aligned with the depth order.
    tile_gathered: GpuAllocation,
    ids_gathered: GpuAllocation,
    /// The tile sort's output: the (tile, depth)-ordered pairs `tile_ranges`
    /// and `blend` read.
    tile_sorted: GpuAllocation,
    ids_sorted: GpuAllocation,
    ranges: GpuAllocation,
    /// The per-frame `SplatView`, rewritten by the host every frame.
    view: GpuAllocation,
}

impl ViewArtifacts {
    /// Allocate the full per-view state; `count` is the view's total
    /// (prepared) Gaussian count, the entry capacity is `count` ×
    /// [`ENTRIES_PER_GAUSSIAN`].
    fn build(
        device: &Device,
        sort_shader: Handle<Shader>,
        sort_prepared: &PreparedShader,
        width: u32,
        height: u32,
        count: u32,
    ) -> Result<Self> {
        let capacity = count.saturating_mul(ENTRIES_PER_GAUSSIAN).max(1);
        let sort = RadixSort::from_prepared(device, sort_prepared, capacity as usize)?;
        let image = {
            let uploader = device.uploader();
            let mut uploader = uploader.lock().unwrap_or_else(|e| e.into_inner());
            Texture::storage_image(
                device,
                &mut uploader,
                width,
                height,
                Format::R16G16B16A16Sfloat,
            )?
        };
        let entry_bytes = u64::from(capacity) * size_of::<u32>() as u64;
        let gpu = |bytes: u64| GpuAllocation::new(device, bytes, MemoryResidency::GpuOnly);
        let slot_identity = GpuAllocation::new(device, entry_bytes, MemoryResidency::CpuWriteOnly)?;
        {
            let host = slot_identity.host().expect("CpuWriteOnly has a host view");
            // SAFETY: the allocation is host-visible, persistently mapped,
            // and sized `capacity` u32s; the identity contents never change.
            unsafe {
                let ptr = host.typed::<u32>();
                for i in 0..capacity {
                    *ptr.add(i as usize) = i;
                }
            }
        }
        Ok(Self {
            extent: (width, height),
            gaussian_count: count,
            capacity,
            sort,
            sort_shader,
            sort_revision: sort_prepared.revision(),
            image,
            transmittance: gpu(u64::from(width) * u64::from(height) * size_of::<f32>() as u64)?,
            records: gpu(u64::from(count) * size_of::<SplatRecord>() as u64)?,
            counters: gpu(2 * size_of::<u32>() as u64)?,
            tile_keys: gpu(entry_bytes)?,
            depth_keys: gpu(entry_bytes)?,
            ids: gpu(entry_bytes)?,
            slot_identity,
            depth_sorted: gpu(entry_bytes)?,
            slots_sorted: gpu(entry_bytes)?,
            tile_gathered: gpu(entry_bytes)?,
            ids_gathered: gpu(entry_bytes)?,
            tile_sorted: gpu(entry_bytes)?,
            ids_sorted: gpu(entry_bytes)?,
            ranges: gpu(u64::from(width.div_ceil(TILE_SIZE) * height.div_ceil(TILE_SIZE)) * 8)?,
            view: GpuAllocation::new(
                device,
                size_of::<SplatView>() as u64,
                MemoryResidency::CpuWriteOnly,
            )?,
        })
    }

    /// Whether these artifacts already serve `(width, height, count)` built
    /// from this sort shader revision.
    fn matches(
        &self,
        width: u32,
        height: u32,
        count: u32,
        sort_shader: Handle<Shader>,
        sort_revision: AssetRevision,
    ) -> bool {
        self.extent == (width, height)
            && self.gaussian_count == count
            && self.sort_shader == sort_shader
            && self.sort_revision == sort_revision
    }

    /// The RGBA16F splat image the chain blends into.
    pub fn image(&self) -> &Texture {
        &self.image
    }

    /// The `(width, height)` the artifacts were built for.
    pub fn extent(&self) -> (u32, u32) {
        self.extent
    }
}

/// Per-view splat forward state, keyed by the camera's [`MainEntity`] (the
/// `ViewTargets` shape: stable across frames, unlike the render-world view
/// entities which rebuild every frame).
#[derive(Default)]
pub struct SplatViewArtifacts {
    views: HashMap<MainEntity, ViewArtifacts>,
}

impl SplatViewArtifacts {
    /// The artifacts for a camera's main entity, if built.
    pub fn get(&self, camera: MainEntity) -> Option<&ViewArtifacts> {
        self.views.get(&camera)
    }

    /// Ensure artifacts serving `(width, height, count)` exist for `camera`,
    /// rebuilding on extent, count, or sort-shader change. Returns false when
    /// the (re)build failed — the caller skips recording rather than running
    /// the chain against mismatched buffers.
    #[allow(clippy::too_many_arguments)]
    fn ensure(
        &mut self,
        camera: MainEntity,
        width: u32,
        height: u32,
        count: u32,
        sort_shader: Handle<Shader>,
        sort_prepared: &PreparedShader,
        device: &Device,
    ) -> bool {
        if self
            .views
            .get(&camera)
            .is_some_and(|a| a.matches(width, height, count, sort_shader, sort_prepared.revision()))
        {
            return true;
        }
        match ViewArtifacts::build(device, sort_shader, sort_prepared, width, height, count) {
            Ok(artifacts) => {
                self.views.insert(camera, artifacts);
                true
            }
            Err(error) => {
                error!("failed to build splat view artifacts: {error}");
                self.views.remove(&camera);
                false
            }
        }
    }

    /// Drop the artifacts of cameras with no extracted view this frame.
    fn retain_cameras(&mut self, claimed: &[MainEntity]) {
        self.views.retain(|camera, _| claimed.contains(camera));
    }
}

/// `PrepareViews` system: retire artifacts of cameras with no view, and
/// (re)build the forward pipelines when the prepared `gaussian.slang` asset
/// advanced past the one they were built from. Both shader requests — the
/// forward chain's and the radix sort's — must be registered; the pass
/// no-ops with a one-shot log otherwise.
pub fn prepare_splat_forward(world: &mut World) {
    let claimed: Vec<MainEntity> = world
        .query::<&ExtractedView>()
        .map(|(_, view)| view.main_entity)
        .collect();
    if let Some(mut artifacts) = world.get_resource_mut::<SplatViewArtifacts>() {
        artifacts.retain_cameras(&claimed);
    }

    let Some(request) = world
        .get_resource::<PipelineShaders>()
        .and_then(|requests| requests.get(SPLAT_FORWARD_SHADER).copied())
    else {
        error_once!(
            "no '{SPLAT_FORWARD_SHADER}' shader registered; skipping the splat forward pass"
        );
        return;
    };
    if world
        .get_resource::<PipelineShaders>()
        .is_none_or(|requests| requests.get(SPLAT_SORT_SHADER).is_none())
    {
        error_once!("no '{SPLAT_SORT_SHADER}' shader registered; skipping the splat forward pass");
        return;
    }
    let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone()) else {
        return;
    };

    let prepared = world.get_resource::<PreparedShaders>();
    let Some(shader) = prepared.as_ref().and_then(|p| p.get(SPLAT_FORWARD_SHADER)) else {
        error_once!("the splat forward shader is not ready; skipping the splat forward pass");
        return;
    };
    let stale = world
        .get_resource::<SplatForwardPipelines>()
        .is_none_or(|pipelines| {
            pipelines.shader != request.shader || pipelines.shader_revision != shader.revision()
        });
    if !stale {
        return;
    }
    let Ok(built) =
        SplatForwardPipelines::from_prepared(render_device.device(), request.shader, shader)
    else {
        return;
    };
    drop(prepared);
    world.insert_resource(built);
}

/// The gs module's camera from the engine's. The engine camera looks down
/// -Z with Y up; the gs module looks down +Z with Y down — the world→view
/// rotation is premultiplied by `diag(1, -1, -1)`, and the focal lengths
/// come from the projection matrix's cotangent terms (m00, m11) scaled to
/// pixels.
fn splat_view(view: &ExtractedView, width: u32, height: u32) -> SplatView {
    let rotation = Mat3::from_mat4(moonfield_camera::view_matrix(&view.world_from_view));
    let w = Mat3::from_diagonal(Vec3::new(1.0, -1.0, -1.0)) * rotation;
    // Slang's `mul(w, v)` contracts with w's rows; glam is column-major, so
    // the transpose's columns are w's rows.
    let rows = w.transpose();
    let cam_pos = view.world_from_view.affine().translation;
    let projection = view
        .camera
        .projection_matrix(width as f32 / height.max(1) as f32);
    SplatView::new(
        [
            rows.x_axis.x,
            rows.x_axis.y,
            rows.x_axis.z,
            rows.y_axis.x,
            rows.y_axis.y,
            rows.y_axis.z,
            rows.z_axis.x,
            rows.z_axis.y,
            rows.z_axis.z,
        ],
        [cam_pos.x, cam_pos.y, cam_pos.z],
        projection.x_axis.x * width as f32 * 0.5,
        projection.y_axis.y * height as f32 * 0.5,
        [width as f32 * 0.5, height as f32 * 0.5],
    )
}

/// Push `params` as `entry`'s one root blob and dispatch `groups`.
fn record<T: bytemuck::Pod>(
    compute: &mut ComputeRecording,
    entry: &ForwardEntry,
    params: &T,
    groups: (u32, u32, u32),
) {
    compute.bind_pipeline(&entry.pipeline);
    compute.push_data(entry.place.offset as u32, bytemuck::bytes_of(params));
    compute.dispatch(groups.0, groups.1, groups.2);
}

/// `Core3d` per-view system, ordered after the opaque pass: ensure the
/// view's artifacts, rewrite the camera record, and record the whole forward
/// chain into the frame command buffer. Clouds without a prepared GPU buffer
/// (still uploading, or failed) are skipped; a view with no prepared clouds
/// records nothing.
pub fn splat_forward(world: &mut World) {
    let Some(view_entity) = world.get_resource::<CurrentView>().map(|view| view.0) else {
        return;
    };
    let Some(view) = world.get_component::<ExtractedView>(view_entity).copied() else {
        return;
    };
    let Some(attachments) = world.get_component::<ViewAttachments>(view_entity) else {
        return;
    };
    let (width, height) = attachments.extent;
    if width == 0 || height == 0 {
        return;
    }

    // The view's prepared clouds: (Gaussian buffer address, count). The base
    // offsets only need internal consistency within this recording.
    let clouds: Vec<(GpuPtr, u32)> = {
        let Some(prepared) = world.get_resource::<PreparedSplats>() else {
            return;
        };
        world
            .query::<&SplatCloudHandle>()
            .filter_map(|(_, handle)| {
                prepared
                    .get(handle.0.id())
                    .map(|cloud| (cloud.gaussians(), cloud.count()))
            })
            .collect()
    };
    let count: u32 = clouds.iter().map(|&(_, n)| n).sum();
    if count == 0 {
        return;
    }
    let Some(render_device) = world.get_resource::<RenderDevice>().map(|d| (*d).clone()) else {
        return;
    };

    // Ensure the artifacts, then rewrite the frame's camera record.
    {
        let Some(sort_request) = world
            .get_resource::<PipelineShaders>()
            .and_then(|requests| requests.get(SPLAT_SORT_SHADER).copied())
        else {
            return;
        };
        let prepared = world.get_resource::<PreparedShaders>();
        let Some(sort_prepared) = prepared.as_ref().and_then(|p| p.get(SPLAT_SORT_SHADER)) else {
            return;
        };
        let Some(mut artifacts) = world.get_resource_mut::<SplatViewArtifacts>() else {
            return;
        };
        if !artifacts.ensure(
            view.main_entity,
            width,
            height,
            count,
            sort_request.shader,
            sort_prepared,
            render_device.device(),
        ) {
            return;
        }
    }
    {
        let splat_view = splat_view(&view, width, height);
        let artifacts = world
            .get_resource::<SplatViewArtifacts>()
            .expect("SplatViewArtifacts ensured above");
        let view_buffer = &artifacts
            .get(view.main_entity)
            .expect("artifacts ensured above")
            .view;
        let host = view_buffer.host().expect("CpuWriteOnly has a host view");
        // SAFETY: the allocation is host-visible, persistently mapped, and
        // sized one SplatView.
        unsafe {
            *host.typed::<SplatView>() = splat_view;
        }
    }

    let Some(pipelines) = world.get_resource::<SplatForwardPipelines>() else {
        return;
    };
    let artifacts = world
        .get_resource::<SplatViewArtifacts>()
        .expect("SplatViewArtifacts ensured above");
    let Some(artifacts) = artifacts.get(view.main_entity) else {
        return;
    };
    let mut ctx = RenderContext::get(world);
    let Some(mut compute) = ctx.compute() else {
        return;
    };
    record_chain(&mut compute, &pipelines, artifacts, &clouds);
}

/// Record the full chain for one view. Every link's read-after-write hazard
/// is the [`ComputeRecording`]'s automatic barrier.
fn record_chain(
    compute: &mut ComputeRecording,
    pipelines: &SplatForwardPipelines,
    artifacts: &ViewArtifacts,
    clouds: &[(GpuPtr, u32)],
) {
    let (width, height) = artifacts.extent;
    let capacity = artifacts.capacity;
    let tiles_x = width.div_ceil(TILE_SIZE);
    let tiles_y = height.div_ceil(TILE_SIZE);
    let tile_count = tiles_x * tiles_y;
    let groups = |n: u32| (n.div_ceil(256), 1, 1);

    let records_ptr = artifacts.records.gpu_ptr();
    let counters_ptr = artifacts.counters.gpu_ptr().as_raw();
    let tile_keys_ptr = artifacts.tile_keys.gpu_ptr().as_raw();
    let depth_keys_ptr = artifacts.depth_keys.gpu_ptr().as_raw();
    let ids_ptr = artifacts.ids.gpu_ptr().as_raw();
    let ranges_ptr = artifacts.ranges.gpu_ptr().as_raw();

    // Sentinel-fill the tile keys and reset the append counters.
    record(
        compute,
        &pipelines.clear,
        &ClearParams::new(tile_keys_ptr, counters_ptr, capacity),
        groups(capacity),
    );

    // Per cloud: project into its slice of the shared record array, then
    // append its tile entries with global (base-shifted) record ids.
    let mut base = 0u32;
    for &(gaussians, count) in clouds {
        let cloud_records = records_ptr
            .offset(u64::from(base) * size_of::<SplatRecord>() as u64)
            .as_raw();
        record(
            compute,
            &pipelines.project_cull,
            &ProjectCullParams::new(
                gaussians.as_raw(),
                cloud_records,
                artifacts.view.gpu_ptr().as_raw(),
                count,
                tiles_x,
                tiles_y,
            ),
            groups(count),
        );
        record(
            compute,
            &pipelines.duplicate_keys,
            &DuplicateKeysParams::new(
                cloud_records,
                counters_ptr,
                tile_keys_ptr,
                depth_keys_ptr,
                ids_ptr,
                count,
                tiles_x,
                capacity,
                base,
            ),
            groups(count),
        );
        base += count;
    }

    // Depth sort over the full capacity (sentinels and all): the values are
    // the constant slot identities, so the output is the permutation.
    artifacts.sort.record(
        compute,
        &artifacts.depth_keys,
        &artifacts.slot_identity,
        &artifacts.depth_sorted,
        &artifacts.slots_sorted,
        capacity,
    );
    // Carry the tile keys and Gaussian ids through that permutation.
    record(
        compute,
        &pipelines.gather,
        &GatherParams::new(
            artifacts.slots_sorted.gpu_ptr().as_raw(),
            tile_keys_ptr,
            ids_ptr,
            artifacts.tile_gathered.gpu_ptr().as_raw(),
            artifacts.ids_gathered.gpu_ptr().as_raw(),
            capacity,
        ),
        groups(capacity),
    );
    // Tile sort (stable): (tile, depth) composite order.
    artifacts.sort.record(
        compute,
        &artifacts.tile_gathered,
        &artifacts.ids_gathered,
        &artifacts.tile_sorted,
        &artifacts.ids_sorted,
        capacity,
    );

    record(
        compute,
        &pipelines.clear_tile_ranges,
        &ClearTileRangesParams::new(ranges_ptr, tile_count),
        groups(tile_count),
    );
    record(
        compute,
        &pipelines.tile_ranges,
        &TileRangesParams::new(
            artifacts.tile_sorted.gpu_ptr().as_raw(),
            ranges_ptr,
            capacity,
        ),
        groups(capacity),
    );

    record(
        compute,
        &pipelines.blend,
        &BlendParams::new(
            records_ptr.as_raw(),
            artifacts.ids_sorted.gpu_ptr().as_raw(),
            ranges_ptr,
            artifacts.transmittance.gpu_ptr().as_raw(),
            artifacts
                .image
                .storage_handle()
                .expect("storage image slot")
                .0,
            width,
            height,
            tiles_x,
        ),
        (tiles_x, tiles_y, 1),
    );
}

/// The splat forward pass as a plugin: the data path (extraction + GPU
/// upload) plus the per-view chain. The app wiring registers both shader
/// requests — [`splat_forward_shader`] and
/// [`crate::splat::sort_pass::splat_sort_shader`] — like the mesh pipeline's.
pub struct SplatForwardPlugin;

impl Plugin for SplatForwardPlugin {
    fn name(&self) -> &str {
        "moonfield_render_feature::splat::SplatForwardPlugin"
    }

    fn build(&self, app: &mut App) {
        app.render_world_mut()
            .insert_resource(PreparedSplats::default());
        app.render_world_mut()
            .insert_resource(SplatViewArtifacts::default());
        app.add_render_systems(
            ExtractSchedule,
            (
                extract_splat_assets,
                extract_with_transform::<SplatCloudHandle>,
            ),
        );
        app.add_render_systems(
            Render,
            prepare_splats.in_set::<render_sets::PrepareAssets>(),
        );
        app.add_render_systems(
            Render,
            prepare_splat_forward.in_set::<render_sets::PrepareViews>(),
        );
        app.add_render_systems(Core3d, splat_forward.after(&opaque_pass_3d));
    }
}

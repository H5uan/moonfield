//! The splat cloud's GPU upload path: pack the CPU SoA `GaussianScene` into
//! the `Gaussian3D` AoS records the forward kernels read, extract referenced
//! clouds into the render world, and keep revision-matched device buffers.
//!
//! Mirrors the mesh module's split (`crate::mesh`): [`extract_splat_assets`]
//! copies referenced `SplatCloud` assets (with their revisions) into the
//! render world — the entities themselves ride the generic
//! `extract_with_transform::<SplatCloudHandle>` — and [`prepare_splats`]
//! (`Render`/`PrepareAssets`) uploads every extracted cloud whose revision
//! advanced into a [`PreparedSplats`] slot.

use std::collections::{HashMap, HashSet};

use moonfield_app::prelude::{App, Commands, IntoSystemConfigs, Plugin, Query, Res, World};
use moonfield_app::{ExtractSchedule, Render};
use moonfield_asset::{AssetId, AssetRevision, Assets, Handle};
use moonfield_render_core::schedule as render_sets;
use moonfield_render_core::{Extract, extract_with_transform};
use moonfield_rhi::{GpuAllocation, GpuPtr, MemoryResidency, RenderDevice};

use crate::splat::cloud::{SplatCloud, SplatCloudHandle};
use crate::splat::forward::Gaussian3D;
use crate::splat::scene::GaussianScene;

/// Pack a scene's SoA attributes into the `Gaussian3D` AoS records the
/// forward kernels index — the byte layout `bytemuck::cast_slice` of the
/// result carries, pinned by the layout test in `splat::forward`. `sh_rest`
/// does not cross: the forward pass evaluates the view-independent DC term
/// only. A scene with mismatched attribute lengths packs to the shortest —
/// the loaders keep them equal; truncating beats panicking in a prepare
/// system.
pub fn pack_gaussians(scene: &GaussianScene) -> Vec<Gaussian3D> {
    let count = scene
        .len()
        .min(scene.scales.len())
        .min(scene.rotations.len())
        .min(scene.opacities.len())
        .min(scene.sh_dc.len());
    (0..count)
        .map(|i| {
            Gaussian3D::new(
                scene.positions[i],
                scene.scales[i],
                scene.rotations[i],
                scene.opacities[i],
                scene.sh_dc[i],
            )
        })
        .collect()
}

/// CPU splat data copied into the render world with the source revision.
pub struct ExtractedSplat {
    /// Revision observed in the main-world asset store.
    pub revision: AssetRevision,
    /// Immutable CPU cloud used to prepare the GPU buffer.
    pub cloud: SplatCloud,
}

/// Referenced splat assets available to render-world preparation systems.
#[derive(Default)]
pub struct ExtractedSplats(HashMap<AssetId, ExtractedSplat>);

impl ExtractedSplats {
    /// Get an extracted splat cloud by asset id.
    pub fn get(&self, id: AssetId) -> Option<&ExtractedSplat> {
        self.0.get(&id)
    }

    /// Iterate all extracted splat clouds.
    pub fn iter(&self) -> impl Iterator<Item = (AssetId, &ExtractedSplat)> {
        self.0.iter().map(|(&id, splat)| (id, splat))
    }

    /// Number of extracted splat assets.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no splat assets are extracted.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The GPU side of one extracted splat cloud: the packed `Gaussian3D` AoS
/// buffer and its length.
pub struct GpuSplatCloud {
    gaussians: GpuAllocation,
    count: u32,
}

impl GpuSplatCloud {
    /// Allocate GPU-only storage and stage the records through the shared
    /// uploader (one flush per frame, ordered ahead of the frame command
    /// buffer by same-queue submission order).
    pub fn new(
        render_device: &RenderDevice,
        gaussians: &[Gaussian3D],
    ) -> moonfield_rhi::Result<Self> {
        let device = render_device.device();
        let uploader = device.uploader();
        let mut uploader = uploader.lock().unwrap_or_else(|e| e.into_inner());
        let allocation = GpuAllocation::new(
            device,
            std::mem::size_of_val(gaussians) as u64,
            MemoryResidency::GpuOnly,
        )?;
        uploader.upload_alloc(&allocation, bytemuck::cast_slice(gaussians))?;
        Ok(Self {
            gaussians: allocation,
            count: gaussians.len() as u32,
        })
    }

    /// The record array's device address — `Ptr<Gaussian3D>` in the shader.
    pub fn gaussians(&self) -> GpuPtr {
        self.gaussians.gpu_ptr()
    }

    /// Number of Gaussians recorded for this cloud.
    pub fn count(&self) -> u32 {
        self.count
    }
}

/// Revision-matched GPU splat clouds owned by the render world.
#[derive(Default)]
pub struct PreparedSplats(HashMap<AssetId, (AssetRevision, GpuSplatCloud)>);

impl PreparedSplats {
    /// Whether `id` is missing or was prepared from an older revision.
    pub fn needs_prepare(&self, id: AssetId, revision: AssetRevision) -> bool {
        self.0
            .get(&id)
            .is_none_or(|(prepared_revision, _)| *prepared_revision != revision)
    }

    /// Insert or replace a prepared cloud for a source revision.
    pub fn insert(
        &mut self,
        id: AssetId,
        revision: AssetRevision,
        cloud: GpuSplatCloud,
    ) -> Option<GpuSplatCloud> {
        self.0
            .insert(id, (revision, cloud))
            .map(|(_, previous)| previous)
    }

    /// Get a prepared cloud by source asset id.
    pub fn get(&self, id: AssetId) -> Option<&GpuSplatCloud> {
        self.0.get(&id).map(|(_, cloud)| cloud)
    }

    /// Remove prepared clouds whose source assets are no longer extracted.
    pub fn retain_extracted(&mut self, extracted: &ExtractedSplats) {
        self.0.retain(|id, _| extracted.0.contains_key(id));
    }

    /// Number of prepared clouds.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no clouds are prepared.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Incrementally copy splat assets referenced by [`SplatCloudHandle`]
/// entities into the render world.
pub fn extract_splat_assets(
    splats: Extract<Query<&SplatCloudHandle>>,
    assets: Extract<Option<Res<Assets<SplatCloud>>>>,
    extracted: Option<Res<ExtractedSplats>>,
    commands: Commands,
) {
    let Some(assets) = assets.as_deref() else {
        commands.insert_resource(ExtractedSplats::default());
        return;
    };

    let mut dangling = 0usize;
    let referenced: HashSet<Handle<SplatCloud>> = splats
        .iter()
        .map(|(_, handle)| handle.0)
        .filter(|handle| {
            if assets.contains(handle) {
                true
            } else {
                dangling += 1;
                false
            }
        })
        .collect();
    if dangling > 0 {
        moonfield_log::warn_once!(
            "{dangling} splat entities reference missing splat assets; they are not rendered"
        );
    }
    let referenced_ids: HashSet<AssetId> = referenced.iter().map(|handle| handle.id()).collect();
    // The updates are precomputed against the current revisions; the render
    // world is only touched once, after this system, through the commands.
    let mut updates = Vec::new();
    for handle in referenced {
        let Some(revision) = assets.revision(&handle) else {
            continue;
        };
        if extracted
            .as_deref()
            .and_then(|current| current.get(handle.id()))
            .is_some_and(|splat| splat.revision == revision)
        {
            continue;
        }
        let Some(cloud) = assets.get(&handle) else {
            continue;
        };
        updates.push((
            handle.id(),
            ExtractedSplat {
                revision,
                cloud: cloud.clone(),
            },
        ));
    }
    commands.queue(move |render_world| {
        let mut extracted = render_world
            .remove_resource::<ExtractedSplats>()
            .unwrap_or_default();
        extracted.0.retain(|id, _| referenced_ids.contains(id));
        for (id, splat) in updates {
            extracted.0.insert(id, splat);
        }
        render_world.insert_resource(extracted);
    });
}

/// Upload changed extracted splat clouds and discard GPU buffers whose source
/// asset is no longer part of the render snapshot.
pub fn prepare_splats(world: &mut World) {
    let Some(render_device) = world
        .get_resource::<RenderDevice>()
        .map(|render_device| (*render_device).clone())
    else {
        return;
    };
    let Some(extracted) = world.get_resource::<ExtractedSplats>() else {
        return;
    };
    let mut prepared = world
        .get_resource_mut::<PreparedSplats>()
        .expect("PreparedSplats registered by SplatForwardPlugin");
    prepared.retain_extracted(&extracted);

    for (id, extracted_splat) in extracted.iter() {
        if extracted_splat.cloud.is_empty() || !prepared.needs_prepare(id, extracted_splat.revision)
        {
            continue;
        }
        let gaussians = pack_gaussians(extracted_splat.cloud.scene());
        match GpuSplatCloud::new(&render_device, &gaussians) {
            Ok(cloud) => {
                prepared.insert(id, extracted_splat.revision, cloud);
            }
            Err(error) => moonfield_log::error!("failed to prepare splat cloud {id:?}: {error}"),
        }
    }
}

/// The splat forward pass's plugin. This is the data-path skeleton: asset
/// and entity extraction plus the GPU upload; the per-view forward chain
/// (kernels from `gs/forward.slang` recorded against each view) lands on top
/// of it.
pub struct SplatForwardPlugin;

impl Plugin for SplatForwardPlugin {
    fn name(&self) -> &str {
        "moonfield_render_feature::splat::SplatForwardPlugin"
    }

    fn build(&self, app: &mut App) {
        app.render_world_mut()
            .insert_resource(PreparedSplats::default());
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_math::GlobalTransform;

    fn sample_scene() -> GaussianScene {
        GaussianScene {
            positions: vec![[1.0, 2.0, 3.0], [-4.0, 5.5, 6.25]],
            scales: vec![[-2.0, -1.5, -1.0], [0.0, 0.25, 0.5]],
            rotations: vec![[1.0, 0.0, 0.0, 0.0], [0.5, -0.5, 0.5, -0.5]],
            opacities: vec![0.0, 1.75],
            sh_dc: vec![[0.1, 0.2, 0.3], [-0.4, 0.5, -0.6]],
            sh_rest: vec![[0.0; 45]; 2],
        }
    }

    #[test]
    fn test_pack_gaussians_round_trips_through_the_mirror() {
        let scene = sample_scene();
        let packed = pack_gaussians(&scene);
        assert_eq!(packed.len(), scene.len());

        let bytes: &[u8] = bytemuck::cast_slice(&packed);
        assert_eq!(bytes.len(), scene.len() * size_of::<Gaussian3D>());
        let records: &[Gaussian3D] = bytemuck::cast_slice(bytes);
        for (i, record) in records.iter().enumerate() {
            assert_eq!(record.mean(), scene.positions[i], "record {i} mean");
            assert_eq!(record.log_scale(), scene.scales[i], "record {i} log_scale");
            assert_eq!(record.rotation(), scene.rotations[i], "record {i} rotation");
            assert_eq!(
                record.logit_opacity(),
                scene.opacities[i],
                "record {i} logit_opacity"
            );
            assert_eq!(record.sh_dc(), scene.sh_dc[i], "record {i} sh_dc");
        }

        // The natural-layout padding (pinned by the reflection test in
        // `splat::forward`) must be zeroed, never garbage: mean pad at
        // 12..16, log_scale pad at 28..32, logit-opacity-to-sh_dc pad at
        // 52..64, trailing pad at 76..80.
        for (i, record) in records.iter().enumerate() {
            let bytes = bytemuck::bytes_of(record);
            for range in [12..16, 28..32, 52..64, 76..80] {
                assert!(
                    bytes[range.clone()].iter().all(|&b| b == 0),
                    "record {i} padding {range:?} must be zero"
                );
            }
        }
    }

    #[test]
    fn test_pack_gaussians_truncates_to_the_shortest_attribute() {
        let mut scene = sample_scene();
        scene.opacities.pop();
        let packed = pack_gaussians(&scene);
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0].mean(), scene.positions[0]);
    }

    /// The extract → prepare flow: referenced clouds land in the render
    /// world (asset resource plus entity components) and, when a device
    /// exists, upload into `PreparedSplats`. Headless machines skip the GPU
    /// half.
    #[test]
    fn test_splat_forward_plugin_extracts_and_prepares_splats() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let render_device = match RenderDevice::new() {
            Ok(render_device) => Some(render_device),
            Err(error) => {
                eprintln!("headless: no Vulkan device available ({error}); extraction only");
                None
            }
        };
        let mut app = App::new();
        // Insert the device first, mirroring the real plugin order (see
        // `plugin.rs`'s prepare test): resources LIFO-drop before the device.
        if let Some(render_device) = render_device {
            app.render_world_mut().insert_resource(render_device);
        }
        let has_device = app.render_world().contains_resource::<RenderDevice>();
        app.add_plugin(crate::RenderFeaturePlugin);
        app.add_plugin(SplatForwardPlugin);

        let cloud = {
            let mut assets = app
                .world_mut()
                .get_resource_mut::<Assets<SplatCloud>>()
                .expect("Assets<SplatCloud> registered by RenderFeaturePlugin");
            assets.add(SplatCloud::new(sample_scene(), None))
        };
        app.world_mut()
            .spawn((SplatCloudHandle(cloud), GlobalTransform::IDENTITY));

        app.render();

        // Asset extraction, revision-matched.
        let revision = app
            .world()
            .get_resource::<Assets<SplatCloud>>()
            .unwrap()
            .revision(&cloud)
            .unwrap();
        let extracted = app
            .render_world()
            .get_resource::<ExtractedSplats>()
            .expect("ExtractedSplats");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted.get(cloud.id()).unwrap().revision, revision);

        // Entity extraction: component + transform + the main-world link.
        let entities: Vec<_> = app
            .render_world()
            .query::<(
                &SplatCloudHandle,
                &GlobalTransform,
                &moonfield_render_core::MainEntity,
            )>()
            .collect();
        assert_eq!(entities.len(), 1);
        let (_, (handle, _, _)) = entities[0];
        assert_eq!(handle.0, cloud);

        if !has_device {
            return;
        }
        let prepared = app
            .render_world()
            .get_resource::<PreparedSplats>()
            .expect("PreparedSplats");
        let gpu = prepared.get(cloud.id()).expect("cloud prepared");
        assert_eq!(gpu.count(), 2);
        assert_ne!(gpu.gaussians().as_raw(), 0);
    }
}

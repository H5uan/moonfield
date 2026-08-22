//! Editor viewport: renders the ECS scene into an offscreen target and
//! exposes it as an egui texture.
//!
//! The scene is queried straight from the [`World`] (the render seam:
//! single-threaded, no extract layer): the entity with
//! [`Camera`] + [`PrimaryCamera`] + `GlobalTransform` provides view and
//! projection (aspect follows the offscreen target's extent), and every
//! entity with [`MeshRenderer`] + `GlobalTransform` is drawn with its mesh
//! from the world's `Assets<Mesh>` resource. Per-draw transform and color go
//! through push constants — no descriptor management.
//!
//! Meshes are uploaded to the GPU lazily: the viewport keeps a
//! `AssetId → GpuMesh` cache and uploads a mesh the first time an entity
//! references it. Splat entities render as a unit-cube AABB placeholder
//! until the real 3DGS rasterizer lands (`splat::rasterize` is still a
//! stub).
//!
//! The offscreen target carries a depth attachment (reverse-Z: clear 0.0,
//! `GREATER_OR_EQUAL`), so overlapping meshes occlude correctly.

use std::collections::HashMap;

use ash::vk;
use moonfield_asset::{AssetId, Assets};
use moonfield_math::{Affine3A, GlobalTransform, Mat4, Quat, Vec3};
use moonfield_render::{
    view_matrix, Buffer, BufferUsage, Camera, CommandBuffer, Compiler, Device, Format,
    GraphicsPipeline, OffscreenTarget, PrimaryCamera, Result, ShaderModule, VertexAttribute,
    VertexBufferLayout, VertexFormat,
};
use moonfield_renderer::mesh::{Mesh, MeshRenderer};
use moonfield_renderer::splat::cloud::{SplatCloud, SplatCloudHandle};

use crate::egui_vk::EguiRenderer;

/// Initial offscreen target size; the viewport panel reports its real size
/// on the first frame.
const INITIAL_WIDTH: u32 = 1280;
const INITIAL_HEIGHT: u32 = 720;

/// Per-draw push constants: model-view-projection matrix + flat color.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScenePushConstants {
    mvp: [f32; 16],
    color: [f32; 4],
}

const PUSH_CONSTANT_SIZE: u32 = std::mem::size_of::<ScenePushConstants>() as u32;

/// A mesh uploaded to the GPU: positions + u32 indices.
struct GpuMesh {
    vertex: Buffer,
    index: Buffer,
    index_count: u32,
}

/// Upload positions + u32 indices into GPU-side buffers.
fn upload_geometry(device: &Device, positions: &[[f32; 3]], indices: &[u32]) -> Result<GpuMesh> {
    let vertex = Buffer::new(
        device,
        std::mem::size_of_val(positions) as u64,
        BufferUsage::VERTEX,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )?;
    vertex.upload(device, positions)?;
    let index = Buffer::new(
        device,
        std::mem::size_of_val(indices) as u64,
        BufferUsage::INDEX,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )?;
    index.upload(device, indices)?;
    Ok(GpuMesh {
        vertex,
        index,
        index_count: indices.len() as u32,
    })
}

/// Which mesh a draw item renders.
#[derive(Clone, Copy)]
enum DrawMesh {
    /// A mesh from the world's `Assets<Mesh>`, resolved through the
    /// viewport's GPU cache.
    Asset(AssetId),
    /// The viewport's internal unit cube (splat AABB placeholder).
    UnitCube,
}

/// One draw in the scene pass: model matrix, flat color, and the mesh.
struct DrawItem {
    model: Mat4,
    color: [f32; 4],
    mesh: DrawMesh,
}

/// The viewport scene: a depth-tested offscreen render target, the flat-lit
/// mesh pipeline, the GPU mesh cache, and the egui texture id pointing at
/// the target.
pub struct Viewport {
    pipeline: GraphicsPipeline,
    /// GPU uploads of world's `Assets<Mesh>` entries, filled on first use.
    gpu_meshes: HashMap<AssetId, GpuMesh>,
    /// The unit cube the splat AABB placeholder draws with.
    unit_cube: GpuMesh,
    target: OffscreenTarget,
    texture_id: Option<egui::TextureId>,
}

impl Viewport {
    /// Create the viewport scene with its initial offscreen target.
    pub fn new(device: &Device) -> Result<Self> {
        let compiler = Compiler::new()?;
        let vertex_spirv =
            compiler.compile_source_to_spirv("viewport_vs", VERTEX_SHADER, "main")?;
        let fragment_spirv =
            compiler.compile_source_to_spirv("viewport_fs", FRAGMENT_SHADER, "main")?;
        let vertex_shader = ShaderModule::from_spirv(device, &vertex_spirv)?;
        let fragment_shader = ShaderModule::from_spirv(device, &fragment_spirv)?;

        let target = OffscreenTarget::new_with_depth(
            device,
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
            Format::B8G8R8A8Unorm,
        )?;
        let pipeline = create_pipeline(device, &target, &vertex_shader, &fragment_shader)?;

        let unit_cube =
            upload_geometry(device, crate::UNIT_CUBE_VERTICES, crate::UNIT_CUBE_INDICES)?;

        Ok(Self {
            pipeline,
            gpu_meshes: HashMap::new(),
            unit_cube,
            target,
            texture_id: None,
        })
    }

    /// Register the offscreen image with the egui renderer (or rebind the
    /// existing id after a [`resize`](Self::resize)). Must be called once
    /// after creation and again after every resize.
    pub fn register_texture(&mut self, device: &Device, egui_renderer: &mut EguiRenderer) {
        let view = self.target.texture_view();
        let sampler = self.target.sampler_view();
        let result = match self.texture_id {
            Some(id) => egui_renderer
                .update_native_texture(device, id, &view, &sampler)
                .map(|_| id),
            None => egui_renderer.register_native_texture(device, &view, &sampler),
        };
        match result {
            Ok(id) => self.texture_id = Some(id),
            Err(e) => moonfield_log::error!("failed to register viewport texture: {e}"),
        }
    }

    /// The egui texture id of the offscreen image, if registered.
    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture_id
    }

    /// The `(width, height)` of the offscreen target.
    pub fn extent(&self) -> (u32, u32) {
        self.target.extent()
    }

    /// Access the offscreen target (debug readback).
    pub fn target(&self) -> &OffscreenTarget {
        &self.target
    }

    /// Resize the offscreen target to match the viewport panel. The pipeline
    /// is untouched: its viewport and scissor are dynamic and follow the
    /// render area. The egui texture rebind happens in
    /// [`register_texture`](Self::register_texture) after the resize.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if (width, height) == self.target.extent() {
            return Ok(());
        }
        self.target.resize(device, width, height)
    }

    /// Record the scene pass into the given command buffer: clear color and
    /// depth, then draw every [`MeshRenderer`] + `GlobalTransform` entity
    /// with its mesh (uploading meshes not yet in the GPU cache), and every
    /// [`SplatCloudHandle`] entity as a unit-cube AABB placeholder.
    ///
    /// With no primary camera in the world, the target is cleared to a dim
    /// placeholder color.
    pub fn record_scene(
        &mut self,
        device: &Device,
        world: &moonfield_ecs::World,
        command_buffer: &CommandBuffer,
    ) {
        // The primary camera: first entity with Camera + PrimaryCamera +
        // GlobalTransform. Copy the data out so later queries don't fight
        // the iteration borrows.
        let mut camera = None;
        for (entity, (cam, global)) in world.query::<(&Camera, &GlobalTransform)>() {
            if world.get_component::<PrimaryCamera>(entity).is_some() {
                camera = Some((*cam, *global));
                break;
            }
        }

        // Collect the draw items up front for the same reason, uploading
        // meshes the cache has not seen yet.
        let mut items: Vec<DrawItem> = Vec::new();
        if let Some(meshes) = world.get_resource::<Assets<Mesh>>() {
            for (_, (renderer, global)) in world.query::<(&MeshRenderer, &GlobalTransform)>() {
                let Some(mesh) = meshes.get(&renderer.mesh.0) else {
                    continue;
                };
                if mesh.indices().is_empty() {
                    continue;
                }
                let id = renderer.mesh.0.id();
                if let std::collections::hash_map::Entry::Vacant(entry) = self.gpu_meshes.entry(id)
                {
                    match upload_geometry(device, mesh.positions(), mesh.indices()) {
                        Ok(gpu) => {
                            entry.insert(gpu);
                        }
                        Err(e) => {
                            moonfield_log::error!("failed to upload mesh {id:?}: {e}");
                            continue;
                        }
                    }
                }
                items.push(DrawItem {
                    model: Mat4::from(global.affine()),
                    color: renderer.color,
                    mesh: DrawMesh::Asset(id),
                });
            }
        }

        // Splat entities render as an axis-aligned placeholder box (the
        // cloud's AABB in a fixed green) until the real 3DGS rasterizer
        // lands — `splat::rasterize` is still a stub.
        if let Some(clouds) = world.get_resource::<Assets<SplatCloud>>() {
            for (_, (handle, global)) in world.query::<(&SplatCloudHandle, &GlobalTransform)>() {
                let Some(cloud) = clouds.get(&handle.0) else {
                    continue;
                };
                let (min, max) = cloud.aabb();
                let (min, max) = (Vec3::from(min), Vec3::from(max));
                let center = (min + max) * 0.5;
                let size = (max - min).max(Vec3::splat(0.01));
                let model = Mat4::from(global.affine())
                    * Mat4::from(Affine3A::from_scale_rotation_translation(
                        size,
                        Quat::IDENTITY,
                        center,
                    ));
                items.push(DrawItem {
                    model,
                    color: [0.3, 0.8, 0.4, 1.0],
                    mesh: DrawMesh::UnitCube,
                });
            }
        }

        let (clear_color, view_proj) = match camera {
            Some((cam, global)) => {
                let (width, height) = self.target.extent();
                let aspect = width as f32 / height.max(1) as f32;
                (
                    cam.clear_color,
                    cam.projection_matrix(aspect) * view_matrix(&global),
                )
            }
            None => ([0.05, 0.0, 0.08, 1.0], Mat4::IDENTITY),
        };

        // Debug seam: MOONFIELD_EDITOR_DEBUG_SCENE=1 logs the scene contents.
        if std::env::var_os("MOONFIELD_EDITOR_DEBUG_SCENE").is_some() {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                let camera_pos = camera.map(|(_, g)| {
                    let t = g.affine().translation;
                    (t.x, t.y, t.z)
                });
                moonfield_log::info!(
                    "scene: camera={camera:?} items={} extent={:?}",
                    items.len(),
                    self.target.extent(),
                    camera = camera_pos,
                );
                moonfield_log::info!("  view_proj: {:?}", view_proj.to_cols_array());
                for item in &items {
                    moonfield_log::info!(
                        "  item mvp: {:?}",
                        (view_proj * item.model).to_cols_array()
                    );
                }
            });
        }

        // Color first, then depth: reverse-Z clears to 0.0 (near → 1).
        let clear_values = [
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: clear_color,
                },
            },
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 0.0,
                    stencil: 0,
                },
            },
        ];
        let (width, height) = self.target.extent();
        let begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.target.render_pass().raw())
            .framebuffer(self.target.framebuffer().raw())
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width, height },
            })
            .clear_values(&clear_values);

        command_buffer.begin_render_pass(&begin_info, vk::SubpassContents::INLINE);
        if camera.is_some() {
            // The engine's projection is Y-up NDC; Vulkan framebuffers are
            // top-left origin. The negative-height viewport performs the flip
            // at the Vulkan boundary (see AGENTS.md clip-space note).
            command_buffer.set_viewport(vk::Viewport {
                x: 0.0,
                y: height as f32,
                width: width as f32,
                height: -(height as f32),
                min_depth: 0.0,
                max_depth: 1.0,
            });
            command_buffer.bind_graphics_pipeline(self.pipeline.raw());
            for item in &items {
                let gpu = match item.mesh {
                    DrawMesh::Asset(id) => &self.gpu_meshes[&id],
                    DrawMesh::UnitCube => &self.unit_cube,
                };
                command_buffer.bind_vertex_buffers(0, &[gpu.vertex.raw()], &[0]);
                command_buffer.bind_index_buffer(gpu.index.raw(), 0, vk::IndexType::UINT32);
                let push = ScenePushConstants {
                    mvp: (view_proj * item.model).to_cols_array(),
                    color: item.color,
                };
                command_buffer.push_constants(
                    self.pipeline.layout(),
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                    0,
                    bytemuck::bytes_of(&push),
                );
                command_buffer.draw_indexed(gpu.index_count, 1, 0, 0, 0);
            }
        }
        command_buffer.end_render_pass();
    }
}

fn create_pipeline(
    device: &Device,
    target: &OffscreenTarget,
    vertex_shader: &ShaderModule,
    fragment_shader: &ShaderModule,
) -> Result<GraphicsPipeline> {
    let vertex_layout = VertexBufferLayout {
        stride: std::mem::size_of::<[f32; 3]>() as u32,
        attributes: vec![VertexAttribute {
            location: 0,
            format: VertexFormat::Float32x3,
            offset: 0,
        }],
    };
    let push_constants = [vk::PushConstantRange {
        stage_flags: vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
        offset: 0,
        size: PUSH_CONSTANT_SIZE,
    }];

    GraphicsPipeline::new_with_options(
        device,
        target.render_pass(),
        vertex_shader,
        fragment_shader,
        &vertex_layout,
        &push_constants,
        &moonfield_render::PipelineOptions {
            cull_mode: moonfield_render::CullMode::None,
            depth_test: true,
            ..Default::default()
        },
    )
}

const VERTEX_SHADER: &str = r#"
struct PushConstants
{
    // Slang packs matrices row-major by default; glam's `to_cols_array` is
    // column-major, so the layout must be declared explicitly.
    column_major float4x4 mvp;
    float4 color;
};

[[vk::push_constant]]
PushConstants push;

struct VsInput
{
    float3 position : POSITION;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float3 local_pos : TEXCOORD0;
};

[shader("vertex")]
VsOutput main(VsInput input)
{
    VsOutput output;
    output.position = mul(push.mvp, float4(input.position, 1.0));
    output.local_pos = input.position;
    return output;
}
"#;

const FRAGMENT_SHADER: &str = r#"
struct PushConstants
{
    column_major float4x4 mvp;
    float4 color;
};

[[vk::push_constant]]
PushConstants push;

struct PsInput
{
    float3 local_pos : TEXCOORD0;
};

[shader("fragment")]
float4 main(PsInput input) : SV_TARGET
{
    // Cheap flat shading: reconstruct the face normal from screen-space
    // derivatives of the local position and light it with a fixed direction.
    float3 normal = normalize(cross(ddx(input.local_pos), ddy(input.local_pos)));
    float3 light_dir = normalize(float3(0.4, 0.8, 0.6));
    float shade = 0.35 + 0.65 * abs(dot(normal, light_dir));
    return float4(push.color.rgb * shade, push.color.a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_math::Transform;
    use moonfield_render::{CommandPool, Instance};
    use moonfield_renderer::mesh::MeshHandle;

    /// A headless Vulkan device, or `None` (test skips) when no driver is
    /// available.
    fn headless_device() -> Option<(Instance, Device)> {
        let instance = match Instance::new_headless() {
            Ok(instance) => instance,
            Err(err) => {
                eprintln!("skipping: no Vulkan instance available ({err})");
                return None;
            }
        };
        match Device::new(&instance, None) {
            Ok(device) => Some((instance, device)),
            Err(err) => {
                eprintln!("skipping: no Vulkan device available ({err})");
                None
            }
        }
    }

    /// The demo scene's camera pose plus one unit-cube entity per
    /// `(color, transform)`, drawn in slice order.
    fn cube_world(cubes: &[([f32; 4], Transform)]) -> moonfield_ecs::World {
        let mut world = moonfield_ecs::World::new();
        world.spawn((
            Camera::default(),
            PrimaryCamera,
            GlobalTransform::from(
                Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
            ),
        ));
        let mut meshes = Assets::<Mesh>::default();
        let cube = MeshHandle(meshes.add(crate::unit_cube_mesh()));
        world.insert_resource(meshes);
        for &(color, transform) in cubes {
            world.spawn((
                MeshRenderer::new(cube, color),
                GlobalTransform::from(transform),
            ));
        }
        world
    }

    /// Record the scene pass and read the target's BGRA pixels back. The
    /// editor's command stream is replicated: a second render pass (the egui
    /// UI pass) follows the scene pass in the same command buffer.
    fn record_and_readback(
        viewport: &mut Viewport,
        device: &Device,
        world: &moonfield_ecs::World,
    ) -> Vec<u8> {
        let command_pool =
            CommandPool::new(device, device.queue_family_indices().graphics).expect("command pool");
        let mut command_buffer = command_pool
            .allocate_command_buffer()
            .expect("command buffer");
        command_buffer
            .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .expect("begin");
        viewport.record_scene(device, world, &command_buffer);

        let ui_target =
            OffscreenTarget::new(device, 64, 64, Format::B8G8R8A8Unorm).expect("ui target");
        let ui_begin = vk::RenderPassBeginInfo::default()
            .render_pass(ui_target.render_pass().raw())
            .framebuffer(ui_target.framebuffer().raw())
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: 64,
                    height: 64,
                },
            })
            .clear_values(&[vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            }]);
        command_buffer.begin_render_pass(&ui_begin, vk::SubpassContents::INLINE);
        command_buffer.end_render_pass();

        // Read the target back in the same submission.
        let (width, height) = viewport.extent();
        let readback = Buffer::new(
            device,
            (width * height * 4) as u64,
            BufferUsage::COPY_DST,
            gpu_allocator::MemoryLocation::GpuToCpu,
        )
        .expect("readback buffer");
        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let to_transfer = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::SHADER_READ)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(viewport.target().image())
            .subresource_range(subresource);
        command_buffer.pipeline_barrier(
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            });
        // SAFETY: the target is in TRANSFER_SRC_OPTIMAL and the buffer fits it.
        unsafe {
            device.raw().cmd_copy_image_to_buffer(
                command_buffer.raw(),
                viewport.target().image(),
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback.raw(),
                std::slice::from_ref(&region),
            );
        }
        command_buffer.end().expect("end");

        let command_buffers = [command_buffer.raw()];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        // SAFETY: the command buffer is fully recorded and the queue is valid.
        unsafe {
            device
                .raw()
                .queue_submit(
                    device.graphics_queue(),
                    std::slice::from_ref(&submit_info),
                    vk::Fence::null(),
                )
                .expect("submit");
            device
                .raw()
                .queue_wait_idle(device.graphics_queue())
                .expect("wait idle");
        }

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        readback.read(&mut pixels).expect("readback");
        pixels
    }

    /// The scene pass must actually rasterize meshes: render a red cube in
    /// front of the primary camera and read the target back.
    #[test]
    fn test_record_scene_draws_cube() {
        let Some((_instance, device)) = headless_device() else {
            return;
        };
        let mut viewport = Viewport::new(&device).expect("viewport");

        let world = cube_world(&[([1.0, 0.0, 0.0, 1.0], Transform::from_xyz(-0.75, 0.0, 0.0))]);
        let pixels = record_and_readback(&mut viewport, &device, &world);

        let (width, height) = viewport.extent();
        if std::env::var_os("MOONFIELD_EDITOR_DEBUG_SCENE").is_some() {
            std::fs::create_dir_all("../../target/tmp").unwrap();
            std::fs::write(
                format!("../../target/tmp/scene_test_{width}x{height}.raw"),
                &pixels,
            )
            .unwrap();
        }
        // Compare against the clear color with rounding tolerance (the GPU
        // rounds unorm values, `(x * 255.0) as u8` truncates).
        let clear = Camera::default().clear_color;
        let is_clear = |px: &[u8]| {
            let (b, g, r, a) = (px[0] as i32, px[1] as i32, px[2] as i32, px[3]);
            (b - (clear[2] * 255.0).round() as i32).abs() <= 1
                && (g - (clear[1] * 255.0).round() as i32).abs() <= 1
                && (r - (clear[0] * 255.0).round() as i32).abs() <= 1
                && a == 255
        };
        let non_clear = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| !is_clear(px.as_slice()))
            .count();
        assert!(
            non_clear > 1000,
            "cube did not rasterize: only {non_clear} non-clear pixels"
        );
    }

    /// Depth direction: a near red cube and a far (larger) blue cube overlap
    /// at screen center; the center pixel must be red even though the blue
    /// cube is drawn second.
    #[test]
    fn test_record_scene_depth_occludes() {
        let Some((_instance, device)) = headless_device() else {
            return;
        };
        let mut viewport = Viewport::new(&device).expect("viewport");

        let world = cube_world(&[
            ([1.0, 0.0, 0.0, 1.0], Transform::from_xyz(0.0, 0.5, 2.0)),
            (
                [0.0, 0.0, 1.0, 1.0],
                Transform {
                    translation: Vec3::new(0.0, 0.5, -3.0),
                    scale: Vec3::splat(4.0),
                    ..Transform::IDENTITY
                },
            ),
        ]);
        let pixels = record_and_readback(&mut viewport, &device, &world);

        let (width, height) = viewport.extent();
        let center = ((height / 2) * width + width / 2) as usize;
        let px = &pixels[center * 4..center * 4 + 4];
        // BGRA: the near red cube wins; the flat shading dims but never
        // swaps channels (red shade ≥ 0.35 → r ≥ 89).
        assert!(
            px[2] > 80 && px[0] < 40 && px[1] < 40,
            "center pixel must be red (near cube occludes), got BGRA {px:?}"
        );
    }
}

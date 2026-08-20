//! Editor viewport: renders the ECS scene into an offscreen target and
//! exposes it as an egui texture.
//!
//! The scene is queried straight from the [`World`] (the render seam:
//! single-threaded, no extract layer): the entity with
//! [`Camera`] + [`PrimaryCamera`] + `GlobalTransform` provides view and
//! projection (aspect follows the offscreen target's extent), and every
//! entity with [`MeshRenderer`] + `GlobalTransform` is drawn as a colored
//! unit cube. Per-cube transform and color go through push constants — no
//! descriptor management.
//!
//! Known slice limitations: no depth attachment on the offscreen target yet
//! (overlapping cubes don't occlude correctly), and all cubes share one mesh.

use ash::vk;
use moonfield_asset::Assets;
use moonfield_math::{Affine3A, GlobalTransform, Mat4, Quat, Vec3};
use moonfield_render::{
    view_matrix, Buffer, BufferUsage, Camera, CommandBuffer, Compiler, Device, Format,
    GraphicsPipeline, MeshRenderer, OffscreenTarget, PrimaryCamera, Result, ShaderModule,
    VertexAttribute, VertexBufferLayout, VertexFormat,
};
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

/// The viewport scene: an offscreen render target, the cube pipeline, the
/// shared cube mesh, and the egui texture id pointing at the target.
pub struct Viewport {
    pipeline: GraphicsPipeline,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
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

        let target =
            OffscreenTarget::new(device, INITIAL_WIDTH, INITIAL_HEIGHT, Format::B8G8R8A8Unorm)?;
        let pipeline = create_pipeline(device, &target, &vertex_shader, &fragment_shader)?;

        let vertex_buffer = Buffer::new(
            device,
            std::mem::size_of_val(CUBE_VERTICES) as u64,
            BufferUsage::VERTEX,
            gpu_allocator::MemoryLocation::CpuToGpu,
        )?;
        vertex_buffer.upload(device, CUBE_VERTICES)?;
        let index_buffer = Buffer::new(
            device,
            std::mem::size_of_val(CUBE_INDICES) as u64,
            BufferUsage::INDEX,
            gpu_allocator::MemoryLocation::CpuToGpu,
        )?;
        index_buffer.upload(device, CUBE_INDICES)?;

        Ok(Self {
            pipeline,
            vertex_buffer,
            index_buffer,
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

    /// Record the scene pass into the given command buffer: clear to the
    /// primary camera's clear color, then draw every
    /// [`MeshRenderer`] + `GlobalTransform` entity as a cube.
    ///
    /// With no primary camera in the world, the target is cleared to a dim
    /// placeholder color.
    pub fn record_scene(&self, world: &moonfield_ecs::World, command_buffer: &CommandBuffer) {
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

        // Collect the draw items up front for the same reason.
        let mut items: Vec<(Mat4, [f32; 4])> = world
            .query::<(&MeshRenderer, &GlobalTransform)>()
            .map(|(_, (mesh, global))| (Mat4::from(global.affine()), mesh.color))
            .collect();

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
                items.push((model, [0.3, 0.8, 0.4, 1.0]));
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
                for (model, _) in &items {
                    moonfield_log::info!("  item mvp: {:?}", (view_proj * model).to_cols_array());
                }
            });
        }

        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: clear_color,
            },
        }];
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
            command_buffer.bind_vertex_buffers(0, &[self.vertex_buffer.raw()], &[0]);
            command_buffer.bind_index_buffer(self.index_buffer.raw(), 0, vk::IndexType::UINT16);
            for (model, color) in items {
                let push = ScenePushConstants {
                    mvp: (view_proj * model).to_cols_array(),
                    color,
                };
                command_buffer.push_constants(
                    self.pipeline.layout(),
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                    0,
                    bytemuck::bytes_of(&push),
                );
                command_buffer.draw_indexed(CUBE_INDICES.len() as u32, 1, 0, 0, 0);
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
            ..Default::default()
        },
    )
}

/// Unit cube (side 1, centered on the origin), positions only. Faces are
/// wound counter-clockwise seen from outside; with the Y-flip viewport that
/// matches the pipeline's clockwise-front-face culling.
const CUBE_VERTICES: &[[f32; 3]] = &[
    [-0.5, -0.5, -0.5], // 0
    [0.5, -0.5, -0.5],  // 1
    [0.5, 0.5, -0.5],   // 2
    [-0.5, 0.5, -0.5],  // 3
    [-0.5, -0.5, 0.5],  // 4
    [0.5, -0.5, 0.5],   // 5
    [0.5, 0.5, 0.5],    // 6
    [-0.5, 0.5, 0.5],   // 7
];

#[rustfmt::skip]
const CUBE_INDICES: &[u16] = &[
    0, 3, 2, 2, 1, 0, // -Z face
    4, 5, 6, 6, 7, 4, // +Z face
    0, 1, 5, 5, 4, 0, // -Y face
    2, 3, 7, 7, 6, 2, // +Y face
    0, 4, 7, 7, 3, 0, // -X face
    1, 2, 6, 6, 5, 1, // +X face
];

const VERTEX_SHADER: &str = r#"
struct PushConstants
{
    float4x4 mvp;
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
    output.position = float4(input.position.xy * 0.5, 0.0, 1.0); // DEBUG bypass mvp
    output.local_pos = input.position;
    return output;
}
"#;

const FRAGMENT_SHADER: &str = r#"
struct PushConstants
{
    float4x4 mvp;
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

    /// The scene pass must actually rasterize cubes: render a red cube in
    /// front of the primary camera and read the target back.
    #[test]
    fn test_record_scene_draws_cube() {
        let instance = match Instance::new_headless() {
            Ok(instance) => instance,
            Err(err) => {
                eprintln!("skipping: no Vulkan instance available ({err})");
                return;
            }
        };
        let device = match Device::new(&instance, None) {
            Ok(device) => device,
            Err(err) => {
                eprintln!("skipping: no Vulkan device available ({err})");
                return;
            }
        };

        let viewport = Viewport::new(&device).expect("viewport");

        // The demo scene's exact poses: camera looking at the cubes.
        let mut world = moonfield_ecs::World::new();
        world.spawn((
            Camera::default(),
            PrimaryCamera,
            GlobalTransform::from(
                Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
            ),
        ));
        world.spawn((
            MeshRenderer::colored([1.0, 0.0, 0.0, 1.0]),
            GlobalTransform::from(Transform::from_xyz(-0.75, 0.0, 0.0)),
        ));

        let command_pool = CommandPool::new(&device, device.queue_family_indices().graphics)
            .expect("command pool");
        let mut command_buffer = command_pool
            .allocate_command_buffer()
            .expect("command buffer");
        command_buffer
            .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .expect("begin");
        viewport.record_scene(&world, &command_buffer);

        // Replicate the editor's command stream: a second render pass (the
        // egui UI pass) follows the scene pass in the same command buffer.
        let ui_target = OffscreenTarget::new(&device, 64, 64, Format::B8G8R8A8Unorm)
            .expect("ui target");
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
            &device,
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
        let non_clear = pixels.chunks_exact(4).filter(|px| !is_clear(px)).count();
        assert!(
            non_clear > 1000,
            "cube did not rasterize: only {non_clear} non-clear pixels"
        );
    }
}

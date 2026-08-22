//! Depth-occlusion test for Lunar Mare's reverse-Z depth path.
//!
//! Renders two fully overlapping quads into an `OffscreenTarget::new_with_depth`
//! target with `PipelineOptions::depth_test` enabled: the near (red) quad is
//! drawn *first*, the far (blue) quad second. Without depth testing the blue
//! quad would overwrite the red one; with reverse-Z depth (clear 0.0, compare
//! `GREATER_OR_EQUAL`) the near quad must win. Skips gracefully on machines
//! without a Vulkan driver.

use ash::vk;
use moonfield_render::{
    Buffer, BufferUsage, CommandPool, Compiler, CullMode, Device, Format, GraphicsPipeline,
    Instance, OffscreenTarget, PipelineOptions, ShaderModule, VertexAttribute, VertexBufferLayout,
    VertexFormat,
};

mod common;
const SIZE: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

/// A full-screen quad (two triangles) at NDC depth `z` with a flat color.
fn quad(z: f32, color: [f32; 3]) -> [Vertex; 6] {
    let corners = [
        [-1.0, -1.0],
        [1.0, -1.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [1.0, 1.0],
        [-1.0, 1.0],
    ];
    corners.map(|[x, y]| Vertex {
        position: [x, y, z],
        color,
    })
}

#[test]
fn depth_test_near_quad_occludes_far_quad() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return;
    }

    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };

    let compiler = Compiler::new().expect("compiler");
    let vertex_spirv = compiler
        .compile_source_to_spirv(
            "quad_vs",
            r#"
struct VsInput
{
    float3 position : POSITION;
    float3 color : COLOR;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float3 color : COLOR;
};

[shader("vertex")]
VsOutput main(VsInput input)
{
    VsOutput output;
    output.position = float4(input.position, 1.0);
    output.color = input.color;
    return output;
}
"#,
            "main",
        )
        .expect("vertex shader");
    let fragment_spirv = compiler
        .compile_source_to_spirv(
            "quad_fs",
            r#"
struct PsInput
{
    float3 color : COLOR;
};

[shader("fragment")]
float4 main(PsInput input) : SV_TARGET
{
    return float4(input.color, 1.0);
}
"#,
            "main",
        )
        .expect("fragment shader");
    let vertex_shader = ShaderModule::from_spirv(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_spirv(&device, &fragment_spirv).expect("fs module");

    let target = OffscreenTarget::new_with_depth(&device, SIZE, SIZE, Format::B8G8R8A8Unorm)
        .expect("target");
    assert!(target.has_depth(), "depth target must report has_depth");

    let pipeline = GraphicsPipeline::new_with_options(
        &device,
        target.render_pass(),
        &vertex_shader,
        &fragment_shader,
        &VertexBufferLayout {
            stride: std::mem::size_of::<Vertex>() as u32,
            attributes: vec![
                VertexAttribute {
                    location: 0,
                    format: VertexFormat::Float32x3,
                    offset: 0,
                },
                VertexAttribute {
                    location: 1,
                    format: VertexFormat::Float32x3,
                    offset: 12,
                },
            ],
        },
        &[],
        &PipelineOptions {
            cull_mode: CullMode::None,
            depth_test: true,
            ..Default::default()
        },
    )
    .expect("pipeline");

    // Reverse-Z: larger NDC z is closer. Draw the near red quad first and the
    // far blue quad second; depth testing must keep red visible.
    let near = quad(0.8, RED);
    let far = quad(0.5, BLUE);
    let vertices = [&near[..], &far[..]].concat();
    let vertex_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(vertices.as_slice()) as u64,
        BufferUsage::VERTEX,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("vertex buffer");
    vertex_buffer
        .upload(&device, vertices.as_slice())
        .expect("upload");

    let readback = Buffer::new(
        &device,
        (SIZE * SIZE * 4) as u64,
        BufferUsage::COPY_DST,
        gpu_allocator::MemoryLocation::GpuToCpu,
    )
    .expect("readback buffer");

    let command_pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut command_buffer = command_pool.allocate_command_buffer().expect("cmd");
    command_buffer
        .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .expect("begin");
    let begin_info = vk::RenderPassBeginInfo::default()
        .render_pass(target.render_pass().raw())
        .framebuffer(target.framebuffer().raw())
        .render_area(vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: SIZE,
                height: SIZE,
            },
        })
        .clear_values(&[
            vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            },
            // Reverse-Z: the far plane is 0, so depth clears to 0.0.
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: 0.0,
                    stencil: 0,
                },
            },
        ]);
    command_buffer.begin_render_pass(&begin_info, vk::SubpassContents::INLINE);
    command_buffer.bind_graphics_pipeline(pipeline.raw());
    command_buffer.bind_vertex_buffers(0, &[vertex_buffer.raw()], &[0]);
    command_buffer.draw(6, 1, 0, 0); // near red quad first
    command_buffer.draw(6, 1, 6, 0); // far blue quad second — must lose
    command_buffer.end_render_pass();

    // Read the target back.
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
        .image(target.image())
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
            width: SIZE,
            height: SIZE,
            depth: 1,
        });
    // SAFETY: the target is in TRANSFER_SRC_OPTIMAL and the buffer fits it.
    unsafe {
        device.raw().cmd_copy_image_to_buffer(
            command_buffer.raw(),
            target.image(),
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

    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    readback.read(&mut pixels).expect("readback");
    let is_red = |px: &[u8; 4]| px[2] > 200 && px[0] < 60 && px[1] < 60 && px[3] == 255;
    let is_blue = |px: &[u8; 4]| px[0] > 200 && px[1] < 60 && px[2] < 60 && px[3] == 255;
    let chunks = pixels.as_chunks::<4>().0;
    let red = chunks.iter().filter(|px| is_red(px)).count();
    let blue = chunks.iter().filter(|px| is_blue(px)).count();

    // The quads cover the whole target; every pixel must be the near red one.
    // Any blue means the far quad won — depth testing is broken.
    assert_eq!(
        blue, 0,
        "far quad leaked through depth test: {blue} blue pixels"
    );
    assert!(
        red > (SIZE * SIZE * 9 / 10) as usize,
        "near quad did not cover the target: {red} red pixels"
    );

    // The depth path must survive a resize (recreates color + depth images).
    target_resize_smoke(&device);
}

/// Resize a depth target and confirm the recreated framebuffer still records.
fn target_resize_smoke(device: &Device) {
    let mut target =
        OffscreenTarget::new_with_depth(device, SIZE, SIZE, Format::B8G8R8A8Unorm).expect("target");
    target.resize(device, 32, 32).expect("resize with depth");
    assert!(target.has_depth());
    assert_eq!(target.extent(), (32, 32));
}

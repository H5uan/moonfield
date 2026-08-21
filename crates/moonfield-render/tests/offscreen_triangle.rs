//! Pixel-verified offscreen draw test for Lunar Mare.
//!
//! `headless_triangle` only checks that commands record; this test actually
//! renders a triangle into an `OffscreenTarget` and reads the pixels back.
//! Skips gracefully on machines without a Vulkan driver.

use ash::vk;
use moonfield_render::{
    Buffer, BufferUsage, CommandPool, Compiler, Device, Format, GraphicsPipeline, Instance,
    OffscreenTarget, ShaderModule, VertexAttribute, VertexBufferLayout, VertexFormat,
};

mod common;
const SIZE: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[test]
fn offscreen_triangle_rasterizes() {
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
            "triangle_vs",
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
            "triangle_fs",
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

    let target = OffscreenTarget::new(&device, SIZE, SIZE, Format::B8G8R8A8Unorm).expect("target");
    let pipeline = GraphicsPipeline::new(
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
    )
    .expect("pipeline");

    // A big triangle covering most of the target.
    let vertices = [
        Vertex {
            position: [-0.8, -0.8, 0.0],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.8, -0.8, 0.0],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.0, 0.8, 0.0],
            color: [1.0, 0.0, 0.0],
        },
    ];
    let vertex_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(&vertices) as u64,
        BufferUsage::VERTEX,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("vertex buffer");
    vertex_buffer.upload(&device, &vertices).expect("upload");

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
        .clear_values(&[vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        }]);
    command_buffer.begin_render_pass(&begin_info, vk::SubpassContents::INLINE);
    command_buffer.bind_graphics_pipeline(pipeline.raw());
    command_buffer.bind_vertex_buffers(0, &[vertex_buffer.raw()], &[0]);
    command_buffer.draw(3, 1, 0, 0);
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
    let red = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[2] > 200 && px[0] < 60 && px[1] < 60 && px[3] == 255)
        .count();
    assert!(red > 100, "triangle did not rasterize: {red} red pixels");
}

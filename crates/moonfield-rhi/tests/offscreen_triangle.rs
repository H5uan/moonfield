//! Pixel-verified offscreen draw test for Lunar Mare.
//!
//! `headless_triangle` only checks that commands record; this test actually
//! renders a triangle into an `OffscreenTarget` and reads the pixels back.
//! Skips gracefully on machines without a Vulkan driver.

use ash::vk;
use moonfield_rhi::{
    AttachmentLayout, Buffer, BufferUsage, ClearValue, CommandBufferUsage, CommandPool, Compiler,
    Device, Format, GraphicsPipeline, Instance, LoadOp, OffscreenTarget, Rect2d, RenderAttachment,
    RenderPassDesc, ShaderModule, StoreOp, VertexAttribute, VertexBufferLayout, VertexFormat,
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
    let vertex_shader = ShaderModule::from_compiled(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_compiled(&device, &fragment_spirv).expect("fs module");

    let target = OffscreenTarget::new(&device, SIZE, SIZE, Format::B8G8R8A8Unorm).expect("target");
    let pipeline = GraphicsPipeline::new(
        &device,
        Format::B8G8R8A8Unorm,
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
        moonfield_rhi::Memory::Default,
    )
    .expect("vertex buffer");
    vertex_buffer.upload(&device, &vertices).expect("upload");

    let command_pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut command_buffer = command_pool.allocate_command_buffer().expect("cmd");
    command_buffer
        .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    let color_attachment = RenderAttachment {
        view: target.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
    };
    let begin_info = RenderPassDesc {
        render_area: Rect2d::full(SIZE, SIZE),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment: None,
    };
    command_buffer.begin_rendering(&begin_info);
    command_buffer.bind_graphics_pipeline(&pipeline);
    command_buffer.bind_vertex_buffers(0, &[&vertex_buffer], &[0]);
    command_buffer.draw(3, 1, 0, 0);
    command_buffer.end_rendering();
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

    let pixels = target.read_pixels(&device).expect("readback");
    let red = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[2] > 200 && px[0] < 60 && px[1] < 60 && px[3] == 255)
        .count();
    assert!(red > 100, "triangle did not rasterize: {red} red pixels");
}

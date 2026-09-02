//! Headless smoke test for Lunar Mare Vulkan RHI.
//!
//! Verifies that instance, device, command pool, command buffer, shader modules,
//! render pass, graphics pipeline, and buffer can be created and that a command
//! buffer can be recorded with a pipeline bind and draw command.

use moonfield_rhi::{
    Buffer, BufferUsage, CommandBufferUsage, CommandPool, Compiler, Device, Format,
    GraphicsPipeline, Instance, ShaderModule, VertexAttribute, VertexBufferLayout, VertexFormat,
};

mod common;
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[test]
fn headless_pipeline_and_command_buffer() {
    // CI runners without the engine's required `VK_EXT_descriptor_heap`
    // (lavapipe, most machines) skip this test; it runs on recent NVIDIA
    // drivers with real hardware.
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

    let compiler = Compiler::new().expect("compiler creation");

    let vertex_source = r#"
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
"#;

    let fragment_source = r#"
struct PsInput
{
    float3 color : COLOR;
};

struct PsOutput
{
    float4 color : SV_TARGET;
};

[shader("fragment")]
PsOutput main(PsInput input)
{
    PsOutput output;
    output.color = float4(input.color, 1.0);
    return output;
}
"#;

    let vertex_spirv = compiler
        .compile_source_to_spirv("triangle_vs", vertex_source, "main")
        .expect("vertex shader compilation");
    let fragment_spirv = compiler
        .compile_source_to_spirv("triangle_fs", fragment_source, "main")
        .expect("fragment shader compilation");

    let vertex_shader =
        ShaderModule::from_compiled(&device, &vertex_spirv).expect("vertex shader module");
    let fragment_shader =
        ShaderModule::from_compiled(&device, &fragment_spirv).expect("fragment shader module");

    let vertex_layout = VertexBufferLayout {
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
                offset: std::mem::size_of::<[f32; 3]>() as u32,
            },
        ],
    };

    let _pipeline = GraphicsPipeline::new(
        &device,
        Format::B8G8R8A8Unorm,
        &vertex_shader,
        &fragment_shader,
        &vertex_layout,
    )
    .expect("graphics pipeline");

    let vertices = [
        Vertex {
            position: [0.0, -0.5, 0.0],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.5, 0.5, 0.0],
            color: [0.0, 1.0, 0.0],
        },
        Vertex {
            position: [-0.5, 0.5, 0.0],
            color: [0.0, 0.0, 1.0],
        },
    ];

    let vertex_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(&vertices) as u64,
        BufferUsage::VERTEX,
        moonfield_rhi::Memory::Default,
    )
    .expect("vertex buffer");
    vertex_buffer
        .upload(&device, &vertices)
        .expect("vertex upload");

    let queue_family_index = device.queue_family_indices().graphics;
    let command_pool = CommandPool::new(&device, queue_family_index).expect("command pool");
    let mut command_buffer = command_pool
        .allocate_command_buffer()
        .expect("command buffer");

    command_buffer
        .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin command buffer");
    command_buffer.bind_graphics_pipeline(&_pipeline);
    command_buffer.bind_vertex_buffers(0, &[&vertex_buffer], &[0]);
    command_buffer.draw(3, 1, 0, 0);
    command_buffer.end().expect("end command buffer");
}

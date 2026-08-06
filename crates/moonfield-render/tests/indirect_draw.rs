//! Headless smoke test for the indirect-draw path of the Lunar Mare Vulkan RHI.
//!
//! Verifies that a `DrawIndirectArgs` buffer can be created with
//! `BufferUsage::INDIRECT` and that a command buffer can record a
//! `draw_indirect` call without panicking. Mirrors `headless_triangle.rs`'s
//! GPU-less skip behavior.

use ash::vk;
use moonfield_render::{
    Buffer, BufferUsage, CommandPool, Compiler, Device, DrawIndirectArgs, Format, GraphicsPipeline,
    Instance, RenderPass, ShaderModule, VertexAttribute, VertexBufferLayout, VertexFormat,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[test]
fn indirect_draw_records_without_panic() {
    // CI runners without a GPU/Vulkan driver (Windows, macOS) skip this test;
    // Linux CI runs it against lavapipe (Mesa software Vulkan).
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
        ShaderModule::from_spirv(&device, &vertex_spirv).expect("vertex shader module");
    let fragment_shader =
        ShaderModule::from_spirv(&device, &fragment_spirv).expect("fragment shader module");

    let render_pass = RenderPass::new(&device, Format::B8G8R8A8Unorm).expect("render pass");

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

    let pipeline = GraphicsPipeline::new(
        &device,
        &render_pass,
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
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("vertex buffer");
    vertex_buffer
        .upload(&device, &vertices)
        .expect("vertex upload");

    // A single indirect draw record: 3 vertices, 1 instance.
    let args = [DrawIndirectArgs {
        vertex_count: 3,
        instance_count: 1,
        first_vertex: 0,
        first_instance: 0,
    }];
    let args_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(&args) as u64,
        BufferUsage::INDIRECT,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("indirect args buffer");
    args_buffer
        .upload(&device, &args)
        .expect("indirect args upload");

    // Sanity: the neutral layout's size matches the Vulkan command struct, so
    // the stride we pass to `draw_indirect` is correct for both backends.
    assert_eq!(
        std::mem::size_of::<DrawIndirectArgs>(),
        std::mem::size_of::<vk::DrawIndirectCommand>(),
    );

    let queue_family_index = device.queue_family_indices().graphics;
    let command_pool = CommandPool::new(&device, queue_family_index).expect("command pool");
    let mut command_buffer = command_pool
        .allocate_command_buffer()
        .expect("command buffer");

    command_buffer
        .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .expect("begin command buffer");
    command_buffer.bind_graphics_pipeline(pipeline.raw());
    command_buffer.bind_vertex_buffers(0, &[vertex_buffer.raw()], &[0]);
    command_buffer.draw_indirect(
        args_buffer.raw(),
        0,
        1,
        std::mem::size_of::<DrawIndirectArgs>() as u32,
    );
    command_buffer.end().expect("end command buffer");

    // Exercise the indexed-indirect API surface too: build a trivial index
    // buffer + indexed args record and record (but do not submit) the command,
    // confirming the binding/indexed-indirect path compiles and records.
    let indices: [u32; 3] = [0, 1, 2];
    let index_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(&indices) as u64,
        BufferUsage::INDEX,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("index buffer");
    index_buffer
        .upload(&device, &indices)
        .expect("index upload");

    let indexed_args = [moonfield_render::DrawIndexedIndirectArgs {
        index_count: 3,
        instance_count: 1,
        first_index: 0,
        base_vertex: 0,
        first_instance: 0,
    }];
    let indexed_args_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(&indexed_args) as u64,
        BufferUsage::INDIRECT,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("indexed indirect args buffer");
    indexed_args_buffer
        .upload(&device, &indexed_args)
        .expect("indexed indirect args upload");

    let mut second = command_pool
        .allocate_command_buffer()
        .expect("second command buffer");
    second
        .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .expect("begin second command buffer");
    second.bind_graphics_pipeline(pipeline.raw());
    second.bind_vertex_buffers(0, &[vertex_buffer.raw()], &[0]);
    second.bind_index_buffer(index_buffer.raw(), 0, vk::IndexType::UINT32);
    second.draw_indexed_indirect(
        indexed_args_buffer.raw(),
        0,
        1,
        std::mem::size_of::<moonfield_render::DrawIndexedIndirectArgs>() as u32,
    );
    second.end().expect("end second command buffer");
}

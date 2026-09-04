//! Headless smoke test for the indirect-draw path of the Lunar Mare Vulkan RHI.
//!
//! Verifies that a `DrawIndirectArgs` buffer can be created with
//! `BufferUsage::INDIRECT` and that command buffers can record
//! `draw_indirect` calls — a single-record draw and a multi-record draw —
//! without panicking. Mirrors `headless_triangle.rs`'s GPU-less skip behavior.

use ash::vk;
use moonfield_rhi::{
    Buffer, BufferUsage, CommandBufferUsage, CommandPool, Compiler, Device, DrawIndirectArgs,
    Format, GpuAllocation, GraphicsPipeline, Instance, Memory, RootBinder, ShaderModule,
};

mod common;
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[test]
fn indirect_draw_records_without_panic() {
    // CI runners without the engine's required `VK_EXT_descriptor_heap`
    // (lavapipe, most drivers) skip this test; mirrors `headless_triangle.rs`.
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

    // Pull-based vertex path: the only stage input is SV_VertexID; geometry is
    // fetched through the `vertices` root pointer (push data, not a bound buffer).
    let vertex_source = r#"
struct VertexData
{
    float3 position;
    float3 color;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float3 color : COLOR;
};

[shader("vertex")]
VsOutput main(uint vid : SV_VertexID, Ptr<VertexData> vertices)
{
    VsOutput output;
    output.position = float4(vertices[vid].position, 1.0);
    output.color = vertices[vid].color;
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

    // The vertex array's device address is delivered through push data; its
    // placement comes from the reflected entry point, not a hand-synced constant.
    let reflection = compiler
        .compile_source_to_reflection("triangle_vs", vertex_source, "main")
        .expect("vertex shader reflection");
    let binder = RootBinder::new(&reflection, "main").expect("root binder");
    let vertices_place = binder.pointer_param("vertices").expect("vertices place");
    drop(reflection);

    let vertex_shader =
        ShaderModule::from_compiled(&device, &vertex_spirv).expect("vertex shader module");
    let fragment_shader =
        ShaderModule::from_compiled(&device, &fragment_spirv).expect("fragment shader module");

    let pipeline = GraphicsPipeline::new(
        &device,
        Format::B8G8R8A8Unorm,
        &vertex_shader,
        &fragment_shader,
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

    let vertex_alloc = GpuAllocation::new(
        &device,
        std::mem::size_of_val(&vertices) as u64,
        Memory::Default,
    )
    .expect("vertex allocation");
    // SAFETY: host-visible allocation, written once before recording.
    unsafe {
        std::ptr::copy_nonoverlapping(
            vertices.as_ptr(),
            vertex_alloc.host().expect("host view").typed::<Vertex>(),
            vertices.len(),
        );
    }

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
        moonfield_rhi::Memory::Default,
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
        .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin command buffer");
    command_buffer.bind_graphics_pipeline(&pipeline);
    // The vertex allocation's address is pushed; the shader pulls vertices
    // through it. One push before the draw: the array does not change.
    let bytes = vertices_place
        .pointer_bytes(vertex_alloc.gpu().as_raw())
        .expect("vertices pointer encode");
    command_buffer.push_data(vertices_place.offset as u32, &bytes);
    command_buffer.draw_indirect(
        &args_buffer,
        0,
        1,
        std::mem::size_of::<DrawIndirectArgs>() as u32,
    );
    command_buffer.end().expect("end command buffer");

    // Exercise the multi-draw side of the args parsing too: two records in
    // one buffer — different vertex counts, one starting mid-array — consumed
    // by a single `draw_indirect` call, confirming the draw-count and stride
    // arithmetic records (but is not submitted).
    let multi_args = [
        DrawIndirectArgs {
            vertex_count: 3,
            instance_count: 1,
            first_vertex: 0,
            first_instance: 0,
        },
        DrawIndirectArgs {
            vertex_count: 2,
            instance_count: 1,
            first_vertex: 1,
            first_instance: 0,
        },
    ];
    let multi_args_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(&multi_args) as u64,
        BufferUsage::INDIRECT,
        moonfield_rhi::Memory::Default,
    )
    .expect("multi-draw args buffer");
    multi_args_buffer
        .upload(&device, &multi_args)
        .expect("multi-draw args upload");

    let mut second = command_pool
        .allocate_command_buffer()
        .expect("second command buffer");
    second
        .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin second command buffer");
    second.bind_graphics_pipeline(&pipeline);
    // Push-data state is per command buffer: the second recording needs its
    // own copy of the vertex array pointer.
    second.push_data(vertices_place.offset as u32, &bytes);
    second.draw_indirect(
        &multi_args_buffer,
        0,
        2,
        std::mem::size_of::<DrawIndirectArgs>() as u32,
    );
    second.end().expect("end second command buffer");
}

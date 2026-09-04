//! Headless smoke test for Lunar Mare Vulkan RHI.
//!
//! Verifies that instance, device, command pool, command buffer, shader modules,
//! render pass, graphics pipeline, and vertex allocation can be created and that
//! a command buffer can be recorded with a pipeline bind, a pushed vertex-array
//! pointer, and a draw command.

use moonfield_rhi::{
    CommandBufferUsage, CommandPool, Compiler, Device, Format, GpuAllocation, GraphicsPipeline,
    Instance, Memory, RootBinder, ShaderModule,
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

    let _pipeline = GraphicsPipeline::new(
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

    let queue_family_index = device.queue_family_indices().graphics;
    let command_pool = CommandPool::new(&device, queue_family_index).expect("command pool");
    let mut command_buffer = command_pool
        .allocate_command_buffer()
        .expect("command buffer");

    command_buffer
        .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin command buffer");
    command_buffer.bind_graphics_pipeline(&_pipeline);
    // One push before the draw: the vertex array does not change during the pass.
    let bytes = vertices_place
        .pointer_bytes(vertex_alloc.gpu().as_raw())
        .expect("vertices pointer encode");
    command_buffer.push_data(vertices_place.offset as u32, &bytes);
    command_buffer.draw(3, 1, 0, 0);
    command_buffer.end().expect("end command buffer");
}

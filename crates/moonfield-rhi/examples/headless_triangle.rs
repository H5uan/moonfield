//! Headless triangle frame recording example.
//!
//! This example creates a minimal Vulkan setup, compiles simple Slang shaders,
//! creates a graphics pipeline, and records a command buffer that draws a
//! triangle. It does not require a window or surface.

use moonfield_rhi::{
    CommandBufferUsage, CommandPool, Compiler, Device, Format, GpuAllocation, GraphicsPipeline,
    Instance, Memory, RootBinder, ShaderModule,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = Instance::new_headless()?;
    let device = Device::new(&instance, None)?;

    let compiler = Compiler::new()?;

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

    let vertex_spirv = compiler.compile_source_to_spirv("triangle_vs", vertex_source, "main")?;
    let fragment_spirv =
        compiler.compile_source_to_spirv("triangle_fs", fragment_source, "main")?;

    // The vertex array's device address is delivered through push data; its
    // placement comes from the reflected entry point, not a hand-synced constant.
    let reflection = compiler.compile_source_to_reflection("triangle_vs", vertex_source, "main")?;
    let binder = RootBinder::new(&reflection, "main")?;
    let vertices_place = binder.pointer_param("vertices")?;
    drop(reflection);

    let vertex_shader = ShaderModule::from_compiled(&device, &vertex_spirv)?;
    let fragment_shader = ShaderModule::from_compiled(&device, &fragment_spirv)?;

    let pipeline = GraphicsPipeline::new(
        &device,
        Format::B8G8R8A8Unorm,
        &vertex_shader,
        &fragment_shader,
    )?;

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
    )?;
    // SAFETY: host-visible allocation, written once before recording.
    unsafe {
        std::ptr::copy_nonoverlapping(
            vertices.as_ptr(),
            vertex_alloc
                .host()
                .ok_or("vertex allocation lost its host view")?
                .typed::<Vertex>(),
            vertices.len(),
        );
    }

    let queue_family_index = device.queue_family_indices().graphics;
    let command_pool = CommandPool::new(&device, queue_family_index)?;
    let mut command_buffer = command_pool.allocate_command_buffer()?;

    command_buffer.begin(CommandBufferUsage::ONE_TIME_SUBMIT)?;

    // In a real swapchain example we would begin a render pass here. For a
    // headless recording demo we bind the pipeline and issue the draw call
    // directly to exercise the command buffer API.
    command_buffer.bind_graphics_pipeline(&pipeline);
    // The vertex allocation's address is pushed; the shader pulls vertices
    // through it. One push before the draw: the array does not change.
    let bytes = vertices_place.pointer_bytes(vertex_alloc.gpu().as_raw())?;
    command_buffer.push_data(vertices_place.offset as u32, &bytes);
    command_buffer.draw(3, 1, 0, 0);

    command_buffer.end()?;

    tracing::info!("Headless triangle frame recorded successfully");
    Ok(())
}

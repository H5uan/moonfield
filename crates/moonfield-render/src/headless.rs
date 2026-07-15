//! Headless one-frame recording utilities.
//!
//! Provides a reusable helper that creates a minimal Vulkan setup, compiles
//! simple Slang shaders, creates a graphics pipeline and vertex buffer, and
//! records a command buffer that draws a triangle.

use crate::error::Result;
use crate::{
    Buffer, CommandBuffer, CommandPool, Compiler, Device, GraphicsPipeline, Instance, RenderPass,
    ShaderModule,
};
use ash::vk;

/// A headless recording context.
///
/// Fields are ordered so that Rust drops them in the correct Vulkan
/// dependency order: child objects first, then device, then instance.
pub struct HeadlessContext {
    #[allow(dead_code)]
    command_buffer: CommandBuffer,
    #[allow(dead_code)]
    command_pool: CommandPool,
    #[allow(dead_code)]
    render_pass: RenderPass,
    #[allow(dead_code)]
    pipeline: GraphicsPipeline,
    #[allow(dead_code)]
    vertex_buffer: Buffer,
    #[allow(dead_code)]
    device: Device,
    #[allow(dead_code)]
    instance: Instance,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

impl HeadlessContext {
    /// Create a headless context and record one frame into a command buffer.
    ///
    /// The command buffer is owned by the returned context and is ready to be
    /// submitted to the graphics queue.
    pub fn record_frame() -> Result<Self> {
        let instance = Instance::new_headless()?;
        let device = Device::new(&instance, None)?;

        let compiler = Compiler::new()?;

        let vertex_spirv =
            compiler.compile_source_to_spirv("triangle_vs", VERTEX_SHADER, "main")?;
        let fragment_spirv =
            compiler.compile_source_to_spirv("triangle_fs", FRAGMENT_SHADER, "main")?;

        let vertex_shader = ShaderModule::from_spirv(&device, &vertex_spirv)?;
        let fragment_shader = ShaderModule::from_spirv(&device, &fragment_spirv)?;

        let render_pass = RenderPass::new(&device, vk::Format::B8G8R8A8_UNORM)?;

        let binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(std::mem::size_of::<Vertex>() as u32)
            .input_rate(vk::VertexInputRate::VERTEX);

        let position_attribute = vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0);

        let color_attribute = vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(1)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(std::mem::size_of::<[f32; 3]>() as u32);

        let extent = vk::Extent2D {
            width: 800,
            height: 600,
        };

        let pipeline = GraphicsPipeline::new(
            &device,
            &render_pass,
            &vertex_shader,
            &fragment_shader,
            &[binding],
            &[position_attribute, color_attribute],
            extent,
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

        let vertex_buffer = Buffer::new(
            &instance,
            &device,
            std::mem::size_of_val(&vertices) as vk::DeviceSize,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        vertex_buffer.upload(&vertices)?;

        let queue_family_index = device.queue_family_indices().graphics;
        let command_pool = CommandPool::new(&device, queue_family_index)?;
        let mut command_buffer = command_pool.allocate_command_buffer()?;

        command_buffer.begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)?;
        command_buffer.bind_graphics_pipeline(pipeline.raw());
        command_buffer.bind_vertex_buffers(0, &[vertex_buffer.raw()], &[0]);
        command_buffer.draw(3, 1, 0, 0);
        command_buffer.end()?;

        Ok(Self {
            instance,
            device,
            render_pass,
            pipeline,
            vertex_buffer,
            command_pool,
            command_buffer,
        })
    }
}

const VERTEX_SHADER: &str = r#"
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

const FRAGMENT_SHADER: &str = r#"
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

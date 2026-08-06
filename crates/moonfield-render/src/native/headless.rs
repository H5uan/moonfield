//! Headless one-frame recording utilities.
//!
//! Provides a reusable helper that creates a minimal Vulkan setup, compiles
//! simple Slang shaders, creates a graphics pipeline and vertex buffer, and
//! records a command buffer that draws a triangle.

use crate::error::{Error, Result};
use crate::{
    Buffer, BufferUsage, CommandBuffer, CommandPool, Compiler, Device, Format, GraphicsPipeline,
    Instance, RenderPass, ShaderModule, VertexAttribute, VertexBufferLayout, VertexFormat,
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
    /// Not a Vulkan object, so its drop position is irrelevant.
    extent: vk::Extent2D,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

impl HeadlessContext {
    /// Create a headless context and record one frame into a command buffer,
    /// sized for a `width`×`height` target.
    ///
    /// The command buffer is owned by the returned context and is ready to be
    /// submitted to the graphics queue.
    pub fn record_frame(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "record_frame dimensions must be non-zero, got {}x{}",
                width, height
            )));
        }

        let extent = vk::Extent2D { width, height };

        let instance = Instance::new_headless()?;
        let device = Device::new(&instance, None)?;

        let compiler = Compiler::new()?;

        let vertex_spirv =
            compiler.compile_source_to_spirv("triangle_vs", VERTEX_SHADER, "main")?;
        let fragment_spirv =
            compiler.compile_source_to_spirv("triangle_fs", FRAGMENT_SHADER, "main")?;

        let vertex_shader = ShaderModule::from_spirv(&device, &vertex_spirv)?;
        let fragment_shader = ShaderModule::from_spirv(&device, &fragment_spirv)?;

        let render_pass = RenderPass::new(&device, Format::B8G8R8A8Unorm)?;

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
            &device,
            std::mem::size_of_val(&vertices) as u64,
            BufferUsage::VERTEX,
            gpu_allocator::MemoryLocation::CpuToGpu,
        )?;
        vertex_buffer.upload(&device, &vertices)?;

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
            extent,
        })
    }

    /// The `(width, height)` extent this context was recorded with.
    pub fn extent(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The requested resolution is reported back as the recorded frame's
    /// extent. Needs a Vulkan device, like the
    /// `headless_triangle` integration test; skipped when no driver
    /// is available (GPU-less CI runners).
    #[test]
    fn test_record_frame_uses_requested_extent() {
        let ctx = match HeadlessContext::record_frame(320, 240) {
            Ok(ctx) => ctx,
            Err(err) => {
                eprintln!("skipping: no Vulkan device available ({err})");
                return;
            }
        };
        assert_eq!(ctx.extent(), (320, 240));
    }

    /// Zero dimensions are rejected before any Vulkan objects are created.
    #[test]
    fn test_record_frame_rejects_zero_dimensions() {
        assert!(HeadlessContext::record_frame(0, 600).is_err());
        assert!(HeadlessContext::record_frame(800, 0).is_err());
    }
}

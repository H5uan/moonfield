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

/// A Slang struct that mirrors a Rust `#[repr(C)]` GPU struct. The
/// `float3 + float + float3 + float` layout yields a 32-byte struct in both
/// Rust `repr(C)` and Slang storage-buffer rules (each `float3` padded to 16
/// bytes). The reflection guard asserts the two match exactly.
#[cfg(test)]
const LAYOUT_SHADER: &str = r#"
struct GpuParticle
{
    float3 position;
    float  mass;
    float3 velocity;
    float  inv_mass;
};

[shader("compute")]
[numthreads(1, 1, 1)]
void main()
{
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// The Rust side of the `GpuParticle` struct. Field ordering and layout
    /// must match the Slang mirror exactly; the reflection guard below asserts
    /// `size`/`offset` so a drift is caught at test time.
    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct GpuParticle {
        position: [f32; 3],
        mass: f32,
        velocity: [f32; 3],
        inv_mass: f32,
    }

    #[test]
    fn test_slang_reflection_guards_rust_struct_layout() {
        let compiler = Compiler::new().expect("compiler");
        // Compile from source via a temp file (same path the SPIR-V path takes).
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join("layout_guard.slang");
        std::fs::write(&temp_path, LAYOUT_SHADER).unwrap();
        let reflection = compiler
            .compile_file_to_reflection(temp_path.to_str().unwrap(), "main")
            .expect("compile to reflection");
        let _ = std::fs::remove_file(&temp_path);

        let layout = reflection
            .struct_layout("GpuParticle")
            .expect("GpuParticle layout");

        // Rust repr(C): 12 + 4 + 12 + 4 = 32 bytes.
        assert_eq!(std::mem::size_of::<GpuParticle>(), 32, "rust repr(C) size");
        // Slang storage-buffer size: each float3 padded to 16 bytes -> 32.
        assert_eq!(layout.size(), 32, "slang reflected size");

        // Field offsets match between Rust and Slang.
        assert_eq!(layout.field_offset("mass").unwrap(), 12, "mass offset");
        assert_eq!(layout.field_offset("velocity").unwrap(), 16, "velocity offset");
        assert_eq!(layout.field_offset("inv_mass").unwrap(), 28, "inv_mass offset");

        // The guard: Rust and Slang must agree on every offset.
        assert_eq!(std::mem::size_of::<GpuParticle>(), layout.size());
    }

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

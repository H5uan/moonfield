//! Editor viewport: renders the scene into an offscreen target and exposes
//! it as an egui texture.

use ash::vk;
use moonfield_render::bind::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingResource, BindingType,
    ShaderStage,
};
use moonfield_render::{
    Buffer, BufferUsage, CommandBuffer, Compiler, Device, Format, GraphicsPipeline,
    OffscreenTarget, Result, ShaderModule, VertexAttribute, VertexBufferLayout, VertexFormat,
};

/// Initial offscreen target size; the viewport panel reports its real size
/// on the first frame.
const INITIAL_WIDTH: u32 = 1280;
const INITIAL_HEIGHT: u32 = 720;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

/// The viewport scene: an offscreen render target, a demo triangle pipeline,
/// and the egui texture bindings pointing at the target.
///
/// Fields are ordered for Vulkan-safe destruction: the bind group and layout
/// first, then the pipeline, then the offscreen target (which waits for
/// device idle). The bind group/layout own their own descriptor objects and
/// drop themselves.
pub struct Viewport {
    bind_group: BindGroup,
    /// Held to keep the layout alive for the lifetime of the bind group;
    /// the bind group references it but does not own it.
    #[allow(dead_code)]
    bind_group_layout: BindGroupLayout,
    pipeline: GraphicsPipeline,
    vertex_buffer: Buffer,
    target: OffscreenTarget,
    texture_id: Option<egui::TextureId>,
}

impl Viewport {
    /// Create the viewport scene with its initial offscreen target.
    pub fn new(device: &Device) -> Result<Self> {
        let compiler = Compiler::new()?;
        let vertex_spirv =
            compiler.compile_source_to_spirv("viewport_vs", VERTEX_SHADER, "main")?;
        let fragment_spirv =
            compiler.compile_source_to_spirv("viewport_fs", FRAGMENT_SHADER, "main")?;
        let vertex_shader = ShaderModule::from_spirv(device, &vertex_spirv)?;
        let fragment_shader = ShaderModule::from_spirv(device, &fragment_spirv)?;

        let target =
            OffscreenTarget::new(device, INITIAL_WIDTH, INITIAL_HEIGHT, Format::B8G8R8A8Unorm)?;
        let pipeline = create_pipeline(device, &target, &vertex_shader, &fragment_shader)?;

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
            device,
            std::mem::size_of_val(&vertices) as u64,
            BufferUsage::VERTEX,
            gpu_allocator::MemoryLocation::CpuToGpu,
        )?;
        vertex_buffer.upload(device, &vertices)?;

        let (bind_group_layout, bind_group) = create_bind_group(device, &target)?;

        Ok(Self {
            bind_group,
            bind_group_layout,
            pipeline,
            vertex_buffer,
            target,
            texture_id: None,
        })
    }

    /// Register the offscreen image as an egui user texture. Must be called
    /// once after creation and again after every [`resize`](Self::resize).
    pub fn register_texture(&mut self, egui_renderer: &mut egui_ash_renderer::Renderer) {
        if let Some(id) = self.texture_id.take() {
            egui_renderer.remove_user_texture(id);
        }
        self.texture_id = Some(egui_renderer.add_user_texture(self.bind_group.raw_vk()));
    }

    /// The egui texture id of the offscreen image, if registered.
    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture_id
    }

    /// The `(width, height)` of the offscreen target.
    pub fn extent(&self) -> (u32, u32) {
        self.target.extent()
    }

    /// Resize the offscreen target to match the viewport panel, recreating
    /// the texture bind group. The pipeline is untouched: its viewport
    /// and scissor are dynamic and follow the render area.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if (width, height) == self.target.extent() {
            return Ok(());
        }
        self.target.resize(device, width, height)?;

        // The bind group references the old image view; recreate it. The
        // target waited for device idle during resize, so the old set is no
        // longer in use; `BindGroup::Drop` frees it from its own pool.
        let (_, bind_group) = create_bind_group(device, &self.target)?;
        self.bind_group = bind_group;
        Ok(())
    }

    /// Record the scene pass into the given command buffer.
    pub fn record_scene(&self, command_buffer: &CommandBuffer) {
        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.02, 0.02, 0.03, 1.0],
            },
        }];
        let (width, height) = self.target.extent();
        let begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(self.target.render_pass().raw())
            .framebuffer(self.target.framebuffer().raw())
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width, height },
            })
            .clear_values(&clear_values);

        command_buffer.begin_render_pass(&begin_info, vk::SubpassContents::INLINE);
        command_buffer.bind_graphics_pipeline(self.pipeline.raw());
        command_buffer.bind_vertex_buffers(0, &[self.vertex_buffer.raw()], &[0]);
        command_buffer.draw(3, 1, 0, 0);
        command_buffer.end_render_pass();
    }
}

fn create_pipeline(
    device: &Device,
    target: &OffscreenTarget,
    vertex_shader: &ShaderModule,
    fragment_shader: &ShaderModule,
) -> Result<GraphicsPipeline> {
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

    GraphicsPipeline::new(
        device,
        target.render_pass(),
        vertex_shader,
        fragment_shader,
        &vertex_layout,
    )
}

fn create_bind_group(
    device: &Device,
    target: &OffscreenTarget,
) -> Result<(BindGroupLayout, BindGroup)> {
    // Borrow neutral views of the target's image view + sampler. The bind
    // group only holds the descriptor set; the underlying view/sampler stay
    // owned by the target.
    let view = target.texture_view();
    let sampler = target.sampler_view();
    let layout = BindGroupLayout::new(
        device,
        &[BindGroupLayoutEntry {
            binding: 0,
            ty: BindingType::SampledTexture,
            visibility: ShaderStage::Fragment,
        }],
    )?;
    let bind_group = BindGroup::new(
        device,
        &layout,
        &[BindGroupEntry {
            binding: 0,
            resource: BindingResource::Texture {
                view: &view,
                sampler: &sampler,
            },
        }],
    )?;
    Ok((layout, bind_group))
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

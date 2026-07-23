//! Editor viewport: renders the scene into an offscreen target and exposes
//! it as an egui texture.

use ash::vk;
use egui_ash_renderer::vulkan::{
    create_vulkan_descriptor_pool, create_vulkan_descriptor_set,
    create_vulkan_descriptor_set_layout,
};
use moonfield_render::{
    Buffer, CommandBuffer, Compiler, Device, Error, GraphicsPipeline, OffscreenTarget, Result,
    ShaderModule,
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
/// Fields are ordered for Vulkan-safe destruction: descriptor bindings and
/// pipeline first, then the offscreen target (which waits for device idle),
/// then the shared device handle.
pub struct Viewport {
    descriptor_set: vk::DescriptorSet,
    descriptor_pool: vk::DescriptorPool,
    descriptor_set_layout: vk::DescriptorSetLayout,
    pipeline: GraphicsPipeline,
    vertex_shader: ShaderModule,
    fragment_shader: ShaderModule,
    vertex_buffer: Buffer,
    target: OffscreenTarget,
    texture_id: Option<egui::TextureId>,
    device: ash::Device,
}

impl Viewport {
    /// Create the viewport scene with its initial offscreen target.
    pub fn new(
        instance: &moonfield_render::Instance,
        device: &Device,
        allocator: std::sync::Arc<std::sync::Mutex<gpu_allocator::vulkan::Allocator>>,
    ) -> Result<Self> {
        let compiler = Compiler::new()?;
        let vertex_spirv =
            compiler.compile_source_to_spirv("viewport_vs", VERTEX_SHADER, "main")?;
        let fragment_spirv =
            compiler.compile_source_to_spirv("viewport_fs", FRAGMENT_SHADER, "main")?;
        let vertex_shader = ShaderModule::from_spirv(device, &vertex_spirv)?;
        let fragment_shader = ShaderModule::from_spirv(device, &fragment_spirv)?;

        let target = OffscreenTarget::new(
            device,
            allocator,
            INITIAL_WIDTH,
            INITIAL_HEIGHT,
            vk::Format::B8G8R8A8_UNORM,
        )?;
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
            instance,
            device,
            std::mem::size_of_val(&vertices) as vk::DeviceSize,
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        vertex_buffer.upload(&vertices)?;

        let (descriptor_set_layout, descriptor_pool, descriptor_set) =
            create_descriptor_bindings(device.raw(), target.image_view(), target.sampler())?;

        Ok(Self {
            descriptor_set,
            descriptor_pool,
            descriptor_set_layout,
            pipeline,
            vertex_shader,
            fragment_shader,
            vertex_buffer,
            target,
            texture_id: None,
            device: device.raw().clone(),
        })
    }

    /// Register the offscreen image as an egui user texture. Must be called
    /// once after creation and again after every [`resize`](Self::resize).
    pub fn register_texture(&mut self, egui_renderer: &mut egui_ash_renderer::Renderer) {
        if let Some(id) = self.texture_id.take() {
            egui_renderer.remove_user_texture(id);
        }
        self.texture_id = Some(egui_renderer.add_user_texture(self.descriptor_set));
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
    /// the pipeline (its viewport is static) and the texture descriptor set.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if (width, height) == self.target.extent() {
            return Ok(());
        }
        self.target.resize(device, width, height)?;
        self.pipeline = create_pipeline(
            device,
            &self.target,
            &self.vertex_shader,
            &self.fragment_shader,
        )?;

        // The descriptor set references the old image view; recreate it.
        // The target waited for device idle during resize, so the old set is
        // no longer in use.
        // SAFETY: the GPU is idle and the set was allocated from our pool.
        unsafe {
            self.device
                .free_descriptor_sets(self.descriptor_pool, &[self.descriptor_set])
                .map_err(|e| Error::Backend(format!("failed to free descriptor set: {:?}", e)))?;
        }
        self.descriptor_set = create_vulkan_descriptor_set(
            &self.device,
            self.descriptor_set_layout,
            self.descriptor_pool,
            self.target.image_view(),
            self.target.sampler(),
        )
        .map_err(|e| Error::Backend(format!("failed to create descriptor set: {e}")))?;
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

impl Drop for Viewport {
    fn drop(&mut self) {
        // SAFETY: the GPU is idle by the time the editor state is dropped
        // (its Drop waits for the device), so these handles are unused.
        unsafe {
            self.device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
        }
    }
}

fn create_pipeline(
    device: &Device,
    target: &OffscreenTarget,
    vertex_shader: &ShaderModule,
    fragment_shader: &ShaderModule,
) -> Result<GraphicsPipeline> {
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

    let (width, height) = target.extent();
    GraphicsPipeline::new(
        device,
        target.render_pass(),
        vertex_shader,
        fragment_shader,
        &[binding],
        &[position_attribute, color_attribute],
        vk::Extent2D { width, height },
    )
}

fn create_descriptor_bindings(
    device: &ash::Device,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
) -> Result<(
    vk::DescriptorSetLayout,
    vk::DescriptorPool,
    vk::DescriptorSet,
)> {
    let layout = create_vulkan_descriptor_set_layout(device)
        .map_err(|e| Error::Backend(format!("failed to create descriptor set layout: {e}")))?;
    let pool = create_vulkan_descriptor_pool(device, 1)
        .map_err(|e| Error::Backend(format!("failed to create descriptor pool: {e}")))?;
    let set = create_vulkan_descriptor_set(device, layout, pool, image_view, sampler)
        .map_err(|e| Error::Backend(format!("failed to create descriptor set: {e}")))?;
    Ok((layout, pool, set))
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

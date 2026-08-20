//! Vulkan graphics pipeline abstraction.

use crate::bind::BindGroupLayout;
use crate::error::{Error, Result};
use crate::types::VertexBufferLayout;
use crate::vulkan::device::Device;
use crate::vulkan::render_pass::RenderPass;
use crate::vulkan::shader_module::ShaderModule;
use ash::vk;

/// How the pipeline's single color attachment blends with existing pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Blending disabled; the fragment color overwrites the target.
    #[default]
    Off,
    /// Premultiplied-alpha blending: color `One, OneMinusSrcAlpha, Add`;
    /// alpha `OneMinusDstAlpha, One, Add`. What egui expects.
    PremultipliedAlpha,
}

impl BlendMode {
    fn to_vk(self) -> vk::PipelineColorBlendAttachmentState {
        let state = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        match self {
            Self::Off => state.blend_enable(false),
            Self::PremultipliedAlpha => state
                .blend_enable(true)
                .src_color_blend_factor(vk::BlendFactor::ONE)
                .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                .color_blend_op(vk::BlendOp::ADD)
                .src_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_DST_ALPHA)
                .dst_alpha_blend_factor(vk::BlendFactor::ONE)
                .alpha_blend_op(vk::BlendOp::ADD),
        }
    }
}

/// Which triangle faces the rasterizer culls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CullMode {
    /// Cull back faces (front face = clockwise, matching the engine's
    /// negative-height viewport convention).
    #[default]
    Back,
    /// No culling — for 2D UI meshes that are all front-facing.
    None,
}

impl CullMode {
    fn to_vk(self) -> vk::CullModeFlags {
        match self {
            Self::Back => vk::CullModeFlags::BACK,
            Self::None => vk::CullModeFlags::NONE,
        }
    }
}

/// Optional pipeline configuration beyond the [`GraphicsPipeline::new`]
/// defaults. `Default` reproduces the pre-options behavior exactly (blend
/// off, back-face culling, no descriptor sets).
#[derive(Default)]
pub struct PipelineOptions<'a> {
    /// Color attachment blend mode.
    pub blend: BlendMode,
    /// Face culling mode.
    pub cull_mode: CullMode,
    /// Descriptor set layouts baked into the pipeline layout (set 0, 1, …).
    /// Borrowed; the caller keeps them alive as long as sets are bound.
    pub set_layouts: &'a [&'a BindGroupLayout],
}

/// A Vulkan graphics pipeline and its layout.
pub struct GraphicsPipeline {
    pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    device: ash::Device,
}

impl GraphicsPipeline {
    /// Create a basic graphics pipeline.
    ///
    /// The pipeline uses the provided vertex/fragment shaders and a single
    /// subpass of `render_pass`. Viewport and scissor are dynamic: they are
    /// set from the render area when a render pass is begun (see
    /// [`CommandBuffer::begin_render_pass`](crate::CommandBuffer::begin_render_pass)),
    /// so the pipeline is independent of the target extent.
    /// `push_constant_ranges` declares the push-constant blocks the shaders
    /// read (empty for none).
    pub fn new(
        device: &Device,
        render_pass: &RenderPass,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
        push_constant_ranges: &[vk::PushConstantRange],
    ) -> Result<Self> {
        Self::new_with_options(
            device,
            render_pass,
            vertex_shader,
            fragment_shader,
            vertex_layout,
            push_constant_ranges,
            &PipelineOptions::default(),
        )
    }

    /// Create a graphics pipeline with explicit [`PipelineOptions`] (blend
    /// mode, cull mode, descriptor set layouts).
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_options(
        device: &Device,
        render_pass: &RenderPass,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
        push_constant_ranges: &[vk::PushConstantRange],
        options: &PipelineOptions,
    ) -> Result<Self> {
        let vertex_entry = std::ffi::CString::new("main").unwrap();
        let fragment_entry = std::ffi::CString::new("main").unwrap();

        let shader_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex_shader.raw())
                .name(&vertex_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment_shader.raw())
                .name(&fragment_entry),
        ];

        let binding = vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(vertex_layout.stride)
            .input_rate(vk::VertexInputRate::VERTEX);
        let attributes: Vec<vk::VertexInputAttributeDescription> = vertex_layout
            .attributes
            .iter()
            .map(|attribute| {
                vk::VertexInputAttributeDescription::default()
                    .binding(0)
                    .location(attribute.location)
                    .format(attribute.format.to_vk())
                    .offset(attribute.offset)
            })
            .collect();

        let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(std::slice::from_ref(&binding))
            .vertex_attribute_descriptions(&attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        // Viewport and scissor are dynamic; only their counts are fixed here.
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(options.cull_mode.to_vk())
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let color_blend_attachments = [options.blend.to_vk()];
        let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&color_blend_attachments);

        let set_layouts: Vec<vk::DescriptorSetLayout> = options
            .set_layouts
            .iter()
            .map(|layout| layout.raw_vk())
            .collect();
        let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(push_constant_ranges)
            .set_layouts(&set_layouts);
        let layout = unsafe {
            device
                .raw()
                .create_pipeline_layout(&pipeline_layout_info, None)
                .map_err(|e| Error::Backend(format!("failed to create pipeline layout: {:?}", e)))?
        };

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_info)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .render_pass(render_pass.raw())
            .subpass(0);

        let pipelines = unsafe {
            device
                .raw()
                .create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    std::slice::from_ref(&pipeline_info),
                    None,
                )
                .map_err(|e| {
                    Error::Backend(format!("failed to create graphics pipeline: {:?}", e))
                })?
        };

        Ok(Self {
            pipeline: pipelines[0],
            layout,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::Pipeline` handle.
    pub fn raw(&self) -> vk::Pipeline {
        self.pipeline
    }

    /// Access the raw `vk::PipelineLayout` handle.
    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }
}

impl Drop for GraphicsPipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.layout, None);
        }
    }
}

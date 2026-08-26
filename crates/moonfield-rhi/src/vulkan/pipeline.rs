//! Vulkan graphics pipeline abstraction.

use crate::bind::BindGroupLayout;
use crate::error::{Error, Result};
use crate::types::{Format, PushConstantRange, VertexBufferLayout};
use crate::vulkan::device::Device;
use crate::vulkan::shader_module::ShaderModule;
use ash::vk;
use ash::vk::TaggedStructure as _;

/// A pipeline layout handle, in the crate's own vocabulary. `Copy` like the
/// underlying Vulkan handle; the owning pipeline keeps it alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineLayout(pub(crate) vk::PipelineLayout);

impl PipelineLayout {
    pub(crate) fn to_vk(self) -> vk::PipelineLayout {
        self.0
    }
}

/// How the pipeline's single color attachment blends with existing pixels.
///
/// Blend is now a dynamic state: [`CommandBuffer::set_blend_state`] applies
/// it per draw, so this type only names the preset used by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Blending disabled; the fragment color overwrites the target.
    #[default]
    Off,
    /// Premultiplied-alpha blending: color `One, OneMinusSrcAlpha, Add`;
    /// alpha `OneMinusDstAlpha, One, Add`. What egui expects.
    PremultipliedAlpha,
}

/// Optional pipeline configuration beyond the [`GraphicsPipeline::new`]
/// defaults: descriptor set layouts. Rasterizer state (blend, cull, depth)
/// is dynamic and applied per draw through the command buffer.
#[derive(Default)]
pub struct PipelineOptions<'a> {
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
    /// Create a basic graphics pipeline targeting a single color attachment.
    ///
    /// The pipeline uses the provided vertex/fragment shaders. Viewport,
    /// scissor, blend, cull, and depth state are all dynamic — they are set
    /// per draw through the command buffer (see
    /// [`CommandBuffer::begin_rendering`](crate::CommandBuffer::begin_rendering)),
    /// so the pipeline is independent of the target extent. Attachment
    /// formats are baked into the pipeline (they affect shader microcode);
    /// `color_format` declares the single color target.
    pub fn new(
        device: &Device,
        color_format: Format,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
        push_constant_ranges: &[PushConstantRange],
    ) -> Result<Self> {
        Self::new_with_options(
            device,
            &[color_format],
            None,
            vertex_shader,
            fragment_shader,
            vertex_layout,
            push_constant_ranges,
            &PipelineOptions::default(),
        )
    }

    /// Create a graphics pipeline with explicit attachment formats and
    /// [`PipelineOptions`] (descriptor set layouts).
    ///
    /// `color_formats` are the color attachment formats the pipeline will be
    /// used against (typically one; multiple for MRT); `depth_format` is the
    /// optional depth attachment format (reverse-Z `D32Sfloat`). These feed
    /// `VkPipelineRenderingCreateInfo`, the dynamic-rendering replacement for
    /// a compatible render pass.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_options(
        device: &Device,
        color_formats: &[Format],
        depth_format: Option<Format>,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
        push_constant_ranges: &[PushConstantRange],
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

        // Viewport, scissor, cull, blend, and depth state are dynamic;
        // only their attachment counts are fixed here. Dynamic states that
        // are merely resets (topology stays TRIANGLE_LIST) can be omitted.
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);

        let dynamic_states = [
            vk::DynamicState::VIEWPORT,
            vk::DynamicState::SCISSOR,
            vk::DynamicState::CULL_MODE,
            vk::DynamicState::FRONT_FACE,
            vk::DynamicState::DEPTH_TEST_ENABLE,
            vk::DynamicState::DEPTH_WRITE_ENABLE,
            vk::DynamicState::DEPTH_COMPARE_OP,
            vk::DynamicState::COLOR_BLEND_ENABLE_EXT,
            vk::DynamicState::COLOR_BLEND_EQUATION_EXT,
            vk::DynamicState::COLOR_WRITE_MASK_EXT,
        ];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);

        // Rasterizer state is entirely dynamic; the values in the create info
        // are ignored for the marked states (kept at sane defaults).
        let rasterizer = vk::PipelineRasterizationStateCreateInfo::default()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        // Depth state is dynamic; the static values are ignored, so depth is
        // left disabled here to make depth-less pipelines valid.
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL)
            .depth_bounds_test_enable(false)
            .stencil_test_enable(false);

        // Blend state is dynamic; the per-attachment array still declares the
        // attachment count (one entry per color format) with inert values.
        let color_blend_attachments: Vec<_> = color_formats
            .iter()
            .map(|_| {
                vk::PipelineColorBlendAttachmentState::default()
                    .color_write_mask(vk::ColorComponentFlags::RGBA)
            })
            .collect();
        let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&color_blend_attachments);

        let set_layouts: Vec<vk::DescriptorSetLayout> = options
            .set_layouts
            .iter()
            .map(|layout| layout.raw_vk())
            .collect();
        let push_constant_ranges_vk: Vec<vk::PushConstantRange> = push_constant_ranges
            .iter()
            .map(|range| range.to_vk())
            .collect();
        let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default()
            .push_constant_ranges(&push_constant_ranges_vk)
            .set_layouts(&set_layouts);
        let layout = unsafe {
            device
                .raw()
                .create_pipeline_layout(&pipeline_layout_info, None)
                .map_err(|e| Error::Backend(format!("failed to create pipeline layout: {:?}", e)))?
        };

        // Dynamic rendering replaces the compatible render pass: formats are
        // baked in through VkPipelineRenderingCreateInfo, and the pipeline
        // names no VkRenderPass (render_pass = VK_NULL_HANDLE, subpass 0).
        let color_formats_vk: Vec<vk::Format> = color_formats.iter().map(|f| f.to_vk()).collect();
        let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&color_formats_vk)
            .depth_attachment_format(depth_format.map_or(vk::Format::UNDEFINED, |f| f.to_vk()));

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_info)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&color_blending)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .subpass(0)
            .render_pass(vk::RenderPass::null())
            .push(&mut rendering_info);

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

    /// The pipeline's layout, for descriptor-set and push-constant commands.
    pub fn layout(&self) -> PipelineLayout {
        PipelineLayout(self.layout)
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

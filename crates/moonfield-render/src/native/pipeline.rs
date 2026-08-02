//! Vulkan graphics pipeline abstraction.

use crate::error::{Error, Result};
use crate::native::device::Device;
use crate::native::render_pass::RenderPass;
use crate::native::shader_module::ShaderModule;
use crate::types::VertexBufferLayout;
use ash::vk;

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
    pub fn new(
        device: &Device,
        render_pass: &RenderPass,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_layout: &VertexBufferLayout,
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
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);

        let color_blend_attachments = [color_blend_attachment];
        let color_blending = vk::PipelineColorBlendStateCreateInfo::default()
            .logic_op_enable(false)
            .attachments(&color_blend_attachments);

        let pipeline_layout_info = vk::PipelineLayoutCreateInfo::default();
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

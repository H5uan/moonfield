//! Vulkan graphics pipeline abstraction.

use crate::device::Device;
use crate::error::{Error, Result};
use crate::render_pass::RenderPass;
use crate::shader_module::ShaderModule;
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
    /// The pipeline uses the provided vertex/fragment shaders, a single
    /// subpass render pass, and static viewport/scissor covering `extent`.
    pub fn new(
        device: &Device,
        render_pass: &RenderPass,
        vertex_shader: &ShaderModule,
        fragment_shader: &ShaderModule,
        vertex_input_bindings: &[vk::VertexInputBindingDescription],
        vertex_input_attributes: &[vk::VertexInputAttributeDescription],
        extent: vk::Extent2D,
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

        let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(vertex_input_bindings)
            .vertex_attribute_descriptions(vertex_input_attributes);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(extent.width as f32)
            .height(extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);

        let scissor = vk::Rect2D::default()
            .offset(vk::Offset2D { x: 0, y: 0 })
            .extent(extent);

        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);

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
            .layout(layout)
            .render_pass(render_pass.raw())
            .subpass(0);

        let pipelines = unsafe {
            device
                .raw()
                .create_graphics_pipelines(vk::PipelineCache::null(), std::slice::from_ref(&pipeline_info), None)
                .map_err(|e| Error::Backend(format!("failed to create graphics pipeline: {:?}", e)))?
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

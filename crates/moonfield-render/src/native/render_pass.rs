//! Vulkan render pass abstraction.

use crate::device::Device;
use crate::error::{Error, Result};
use ash::vk;

/// A Vulkan render pass with a single color attachment.
pub struct RenderPass {
    render_pass: vk::RenderPass,
    device: ash::Device,
}

impl RenderPass {
    /// Create a simple render pass for the given color attachment format.
    ///
    /// The attachment's final layout is `PRESENT_SRC_KHR`, suitable for
    /// rendering directly into a swapchain image.
    pub fn new(device: &Device, color_format: vk::Format) -> Result<Self> {
        Self::new_with_final_layout(device, color_format, vk::ImageLayout::PRESENT_SRC_KHR)
    }

    /// Create a simple render pass with an explicit final layout for the
    /// color attachment (e.g. `SHADER_READ_ONLY_OPTIMAL` for offscreen
    /// targets that are sampled afterwards).
    pub fn new_with_final_layout(
        device: &Device,
        color_format: vk::Format,
        final_layout: vk::ImageLayout,
    ) -> Result<Self> {
        let color_attachment = vk::AttachmentDescription::default()
            .format(color_format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(final_layout);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_attachment_ref));

        let dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

        let attachments = [color_attachment];
        let subpasses = [subpass];
        // When the attachment is sampled after the pass (offscreen targets),
        // add an external dependency so the layout transition to
        // SHADER_READ_ONLY_OPTIMAL is synchronized with fragment shader reads.
        let mut dependencies = vec![dependency];
        if final_layout == vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL {
            dependencies.push(
                vk::SubpassDependency::default()
                    .src_subpass(0)
                    .dst_subpass(vk::SUBPASS_EXTERNAL)
                    .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                    .dst_stage_mask(vk::PipelineStageFlags::FRAGMENT_SHADER)
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ),
            );
        }

        let create_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);

        let render_pass = unsafe {
            device
                .raw()
                .create_render_pass(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create render pass: {:?}", e)))?
        };

        Ok(Self {
            render_pass,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::RenderPass` handle.
    pub fn raw(&self) -> vk::RenderPass {
        self.render_pass
    }
}

impl Drop for RenderPass {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_render_pass(self.render_pass, None);
        }
    }
}

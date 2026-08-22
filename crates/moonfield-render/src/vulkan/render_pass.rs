//! Vulkan render pass abstraction.

use crate::error::{Error, Result};
use crate::types::Format;
use crate::vulkan::device::Device;
use ash::vk;

/// A Vulkan render pass with a single color attachment and, optionally, a
/// depth attachment (`D32Sfloat`).
pub struct RenderPass {
    render_pass: vk::RenderPass,
    device: ash::Device,
    has_depth: bool,
}

impl RenderPass {
    /// Create a simple render pass for the given color attachment format.
    ///
    /// The attachment's final layout is `PRESENT_SRC_KHR`, suitable for
    /// rendering directly into a swapchain image.
    pub fn new(device: &Device, color_format: Format) -> Result<Self> {
        Self::new_with_final_layout(device, color_format, vk::ImageLayout::PRESENT_SRC_KHR)
    }

    /// Create a simple render pass with an explicit final layout for the
    /// color attachment (e.g. `SHADER_READ_ONLY_OPTIMAL` for offscreen
    /// targets that are sampled afterwards).
    ///
    /// `final_layout` is a raw Vulkan layout for now: this type is part of
    /// the Vulkan backend, and a neutral layout enum keeps the public API
    /// backend.
    pub fn new_with_final_layout(
        device: &Device,
        color_format: Format,
        final_layout: vk::ImageLayout,
    ) -> Result<Self> {
        Self::create(device, color_format, final_layout, false)
    }

    /// Create a render pass with an additional depth attachment
    /// (`D32Sfloat`, attachment index 1).
    ///
    /// The engine uses reverse-Z (near → 1, far → 0), so the depth attachment
    /// clears to 0.0; pair it with a `GREATER_OR_EQUAL` depth compare (see
    /// [`PipelineOptions::depth_test`](crate::vulkan::pipeline::PipelineOptions)).
    /// The depth attachment's store op is `DONT_CARE` and its final layout is
    /// `DEPTH_STENCIL_ATTACHMENT_OPTIMAL`. A pass begun against this render
    /// pass must supply two clear values: color first, then depth 0.0.
    pub fn new_with_depth(
        device: &Device,
        color_format: Format,
        final_layout: vk::ImageLayout,
    ) -> Result<Self> {
        Self::create(device, color_format, final_layout, true)
    }

    fn create(
        device: &Device,
        color_format: Format,
        final_layout: vk::ImageLayout,
        with_depth: bool,
    ) -> Result<Self> {
        let color_attachment = vk::AttachmentDescription::default()
            .format(color_format.to_vk())
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(final_layout);

        let depth_attachment = vk::AttachmentDescription::default()
            .format(vk::Format::D32_SFLOAT)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

        let color_attachment_ref = vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

        let depth_attachment_ref = vk::AttachmentReference::default()
            .attachment(1)
            .layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(std::slice::from_ref(&color_attachment_ref));
        let subpass = if with_depth {
            subpass.depth_stencil_attachment(&depth_attachment_ref)
        } else {
            subpass
        };

        let mut dependency = vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);
        if with_depth {
            dependency = dependency
                .src_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                        | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                )
                .dst_stage_mask(
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                        | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                        | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                )
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                        | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                );
        }

        let attachments = if with_depth {
            vec![color_attachment, depth_attachment]
        } else {
            vec![color_attachment]
        };
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
            has_depth: with_depth,
        })
    }

    /// Access the raw `vk::RenderPass` handle.
    pub fn raw(&self) -> vk::RenderPass {
        self.render_pass
    }

    /// Whether this render pass has a depth attachment. When true, a begun
    /// pass must supply two clear values (color, then depth 0.0) and its
    /// framebuffer must include a depth view as attachment 1.
    pub fn has_depth(&self) -> bool {
        self.has_depth
    }
}

impl Drop for RenderPass {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_render_pass(self.render_pass, None);
        }
    }
}

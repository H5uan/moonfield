//! Vulkan framebuffer abstraction.

use crate::device::Device;
use crate::error::{Error, Result};
use crate::render_pass::RenderPass;
use ash::vk;

/// A Vulkan framebuffer.
pub struct Framebuffer {
    framebuffer: vk::Framebuffer,
    device: ash::Device,
}

impl Framebuffer {
    /// Create a framebuffer compatible with the given render pass and attachments.
    pub fn new(
        device: &Device,
        render_pass: &RenderPass,
        attachments: &[vk::ImageView],
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let create_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass.raw())
            .attachments(attachments)
            .width(extent.width)
            .height(extent.height)
            .layers(1);

        let framebuffer = unsafe {
            device
                .raw()
                .create_framebuffer(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create framebuffer: {:?}", e)))?
        };

        Ok(Self {
            framebuffer,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::Framebuffer` handle.
    pub fn raw(&self) -> vk::Framebuffer {
        self.framebuffer
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_framebuffer(self.framebuffer, None);
        }
    }
}

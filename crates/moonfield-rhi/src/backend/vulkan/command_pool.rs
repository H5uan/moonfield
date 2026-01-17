use crate::{types::*, *};
use ash::vk::Handle;
use std::sync::Arc;

use super::{VulkanSwapchain, VulkanCommandBuffer};

pub struct VulkanCommandPool {
    pub device: ash::Device,
    pub command_pool: ash::vk::CommandPool,
    pub swapchain: std::sync::Weak<VulkanSwapchain>,
}

impl CommandPool for VulkanCommandPool {
    fn allocate_command_buffer(&self) -> Result<Arc<dyn CommandBuffer>, RhiError> {
        unsafe {
            let alloc_info = ash::vk::CommandBufferAllocateInfo::default()
                .command_pool(self.command_pool)
                .level(ash::vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);

            let command_buffers = self
                .device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to allocate command buffers: {}", e)))?;

            Ok(Arc::new(VulkanCommandBuffer {
                device: self.device.clone(),
                command_buffer: command_buffers[0],
                swapchain: Some(self.swapchain.clone()),
                current_image_index: std::cell::Cell::new(None),
            }))
        }
    }
}

impl Drop for VulkanCommandPool {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            self.device.destroy_command_pool(self.command_pool, None);
        }
    }
}
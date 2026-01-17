use crate::{types::*, *};
use ash::vk::Handle;
use std::sync::Arc;

pub struct VulkanBuffer {
    pub device: ash::Device,
    pub buffer: ash::vk::Buffer,
    pub memory: ash::vk::DeviceMemory,
    pub size: u64,
}

impl Buffer for VulkanBuffer {
    fn map(&self) -> Result<*mut u8, RhiError> {
        unsafe {
            self.device
                .map_memory(self.memory, 0, self.size, ash::vk::MemoryMapFlags::empty())
                .map(|ptr| ptr as *mut u8)
                .map_err(|e| RhiError::MapFailed(format!("Failed to map buffer memory: {}", e)))
        }
    }

    fn unmap(&self) {
        unsafe {
            self.device.unmap_memory(self.memory);
        }
    }
}

impl Drop for VulkanBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
            self.device.free_memory(self.memory, None);
        }
    }
}
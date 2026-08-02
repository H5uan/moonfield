//! Vulkan synchronization primitives.

use crate::device::Device;
use crate::error::{Error, Result};
use ash::vk;

/// A Vulkan semaphore.
pub struct Semaphore {
    semaphore: vk::Semaphore,
    device: ash::Device,
}

impl Semaphore {
    /// Create a new binary semaphore.
    pub fn new(device: &Device) -> Result<Self> {
        let create_info = vk::SemaphoreCreateInfo::default();
        let semaphore = unsafe {
            device
                .raw()
                .create_semaphore(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create semaphore: {:?}", e)))?
        };

        Ok(Self {
            semaphore,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::Semaphore` handle.
    pub fn raw(&self) -> vk::Semaphore {
        self.semaphore
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_semaphore(self.semaphore, None);
        }
    }
}

/// A Vulkan fence.
pub struct Fence {
    fence: vk::Fence,
    device: ash::Device,
}

impl Fence {
    /// Create a new fence.
    pub fn new(device: &Device, signaled: bool) -> Result<Self> {
        let flags = if signaled {
            vk::FenceCreateFlags::SIGNALED
        } else {
            vk::FenceCreateFlags::empty()
        };
        let create_info = vk::FenceCreateInfo::default().flags(flags);
        let fence = unsafe {
            device
                .raw()
                .create_fence(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create fence: {:?}", e)))?
        };

        Ok(Self {
            fence,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::Fence` handle.
    pub fn raw(&self) -> vk::Fence {
        self.fence
    }

    /// Wait for the fence to be signaled.
    pub fn wait(&self, timeout_ns: u64) -> Result<()> {
        unsafe {
            self.device
                .wait_for_fences(std::slice::from_ref(&self.fence), true, timeout_ns)
                .map_err(|e| Error::Backend(format!("failed to wait for fence: {:?}", e)))?;
        }
        Ok(())
    }

    /// Reset the fence to unsignaled.
    pub fn reset(&self) -> Result<()> {
        unsafe {
            self.device
                .reset_fences(std::slice::from_ref(&self.fence))
                .map_err(|e| Error::Backend(format!("failed to reset fence: {:?}", e)))?;
        }
        Ok(())
    }
}

impl Drop for Fence {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_fence(self.fence, None);
        }
    }
}

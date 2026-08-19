//! Vulkan synchronization primitives.

use crate::error::{Error, Result};
use crate::vulkan::device::Device;
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

    pub fn new_timeline(device: &Device, initial_value: u64) -> Result<Self> {
        let mut type_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(initial_value);
        let create_info = vk::SemaphoreCreateInfo::default().push_next(&mut type_info);
        let semaphore = unsafe {
            device
                .raw()
                .create_semaphore(&create_info, None)
                .map_err(|e| {
                    Error::Backend(format!("failed to create timeline semaphore: {:?}", e))
                })?
        };
        Ok(Self {
            semaphore,
            device: device.raw().clone(),
        })
    }

    /// Block the CPU until the timeline counter reaches at least `value`.
    pub fn wait(&self, value: u64, timeout_ns: u64) -> Result<()> {
        let wait_info = vk::SemaphoreWaitInfo::default()
            .semaphores(std::slice::from_ref(&self.semaphore))
            .values(std::slice::from_ref(&value));
        unsafe {
            self.device
                .wait_semaphores(&wait_info, timeout_ns)
                .map_err(|e| {
                    Error::Backend(format!("failed to wait for timeline semaphore: {:?}", e))
                })?;
        }
        Ok(())
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

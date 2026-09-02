//! Vulkan synchronization primitives.

use crate::error::{Error, Result};
use crate::vulkan::device::Device;
use ash::vk::{self, TaggedStructure as _};

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
        let create_info = vk::SemaphoreCreateInfo::default().push(&mut type_info);
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

/// A GPU pipeline stage mask for bindless barriers.
///
/// Bindless synchronization is stage-to-stage: the barrier orders the end of
/// a producer stage against the start of a consumer stage, without naming any
/// resource — shaders address memory indirectly through pointers, so a
/// resource list would be both impossible and meaningless. The access mask is
/// the widest possible read/write, matching the pointer model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stage(pub(crate) vk::PipelineStageFlags2);

impl Stage {
    /// Vertex shader stage
    pub const VERTEX: Self = Self(vk::PipelineStageFlags2::VERTEX_SHADER);
    /// Fragment shader stage
    pub const FRAGMENT: Self = Self(vk::PipelineStageFlags2::FRAGMENT_SHADER);
    /// Compute shader stage (dispatch).
    pub const COMPUTE: Self = Self(vk::PipelineStageFlags2::COMPUTE_SHADER);
    /// Transfer stage (buffer/image copy).
    pub const TRANSFER: Self = Self(vk::PipelineStageFlags2::TRANSFER);
    /// All stages; implies the widest dependency and ignores access masks.
    pub const ALL: Self = Self(vk::PipelineStageFlags2::ALL_COMMANDS);

    pub(crate) fn to_vk(self) -> vk::PipelineStageFlags2 {
        self.0
    }
}

impl std::ops::BitOr for Stage {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// What kind of hazard a barrier orders — the blog's barrier flags. A plain
/// memory hazard covers pointer-accessed data; a descriptor hazard additionally
/// exposes the descriptor read the next stage performs through non-uniform
/// heap indices (a sampled image read).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarrierHazard {
    /// Plain memory read/write hazard (current behavior).
    #[default]
    Memory,
    /// Descriptor-heap hazard: a stage (or the CPU, through the host mapping)
    /// just wrote heap descriptors that the next stage samples.
    Descriptors,
}

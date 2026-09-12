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
        // SAFETY: the device is valid and the default create info describes a
        // legal binary semaphore.
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
    pub(crate) fn raw(&self) -> vk::Semaphore {
        self.semaphore
    }

    pub fn new_timeline(device: &Device, initial_value: u64) -> Result<Self> {
        let mut type_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(initial_value);
        let create_info = vk::SemaphoreCreateInfo::default().push(&mut type_info);
        // SAFETY: the device is valid and the type-chained create info
        // describes a legal timeline semaphore; `type_info` outlives the call.
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
        // SAFETY: the semaphore is a live timeline semaphore owned by `self`,
        // and the wait info's semaphore and value slices have matching lengths.
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
        // SAFETY: the semaphore was created by this device and is destroyed
        // exactly once, here.
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
        // SAFETY: the device is valid and the create info describes a legal
        // fence.
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
    pub(crate) fn raw(&self) -> vk::Fence {
        self.fence
    }

    /// Wait for the fence to be signaled.
    pub fn wait(&self, timeout_ns: u64) -> Result<()> {
        // SAFETY: the fence is live and owned by `self`; waiting is valid in
        // any fence state.
        unsafe {
            self.device
                .wait_for_fences(std::slice::from_ref(&self.fence), true, timeout_ns)
                .map_err(|e| Error::Backend(format!("failed to wait for fence: {:?}", e)))?;
        }
        Ok(())
    }

    /// Reset the fence to unsignaled.
    pub fn reset(&self) -> Result<()> {
        // SAFETY: the fence is live and owned by `self`; callers reset only an
        // unsignaled fence with no pending submissions, as Vulkan requires.
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
        // SAFETY: the fence was created by this device and is destroyed exactly
        // once, here.
        unsafe {
            self.device.destroy_fence(self.fence, None);
        }
    }
}

/// A GPU pipeline stage mask for bindless barriers.
///
/// Bindless synchronization is stage-to-stage: a barrier orders the end of a
/// producer stage against the start of a consumer stage, without naming any
/// resource — shaders address memory indirectly through pointers, so a
/// resource list would be both impossible and meaningless. The paired
/// [`Access`] masks name which memory operations each side performs.
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
    /// Every graphics stage — the honest widening for a render pass, whose
    /// work spans shader, fragment-test, and attachment stages.
    pub const ALL_GRAPHICS: Self = Self(vk::PipelineStageFlags2::ALL_GRAPHICS);
    /// All stages; implies the widest dependency.
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

/// A GPU access mask for bindless barriers — the hazard half of a [`Stage`]
/// pair: which memory operations the producer finished and which the consumer
/// will perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access(pub(crate) vk::AccessFlags2);

impl Access {
    /// No access (a pure execution dependency).
    pub const NONE: Self = Self(vk::AccessFlags2::NONE);
    /// Indirect argument/dispatch record read.
    pub const INDIRECT_COMMAND_READ: Self = Self(vk::AccessFlags2::INDIRECT_COMMAND_READ);
    /// Shader read through a device address (`GpuPtr`) or storage buffer.
    pub const SHADER_READ: Self = Self(vk::AccessFlags2::SHADER_READ);
    /// Shader write through a device address, storage buffer, or storage image.
    pub const SHADER_WRITE: Self = Self(vk::AccessFlags2::SHADER_WRITE);
    /// Sampled-image read through a descriptor-heap slot.
    pub const SHADER_SAMPLED_READ: Self = Self(vk::AccessFlags2::SHADER_SAMPLED_READ);
    /// Color attachment read (load op, blending).
    pub const COLOR_ATTACHMENT_READ: Self = Self(vk::AccessFlags2::COLOR_ATTACHMENT_READ);
    /// Color attachment write.
    pub const COLOR_ATTACHMENT_WRITE: Self = Self(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);
    /// Depth/stencil attachment read (depth test).
    pub const DEPTH_STENCIL_READ: Self = Self(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ);
    /// Depth/stencil attachment write.
    pub const DEPTH_STENCIL_WRITE: Self = Self(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE);
    /// Transfer read (copy/blit source).
    pub const TRANSFER_READ: Self = Self(vk::AccessFlags2::TRANSFER_READ);
    /// Transfer write (copy/blit destination).
    pub const TRANSFER_WRITE: Self = Self(vk::AccessFlags2::TRANSFER_WRITE);
    /// Any memory read.
    pub const MEMORY_READ: Self = Self(vk::AccessFlags2::MEMORY_READ);
    /// Any memory write.
    pub const MEMORY_WRITE: Self = Self(vk::AccessFlags2::MEMORY_WRITE);
    /// The widest mask: every read and write.
    pub const ALL: Self = Self(vk::AccessFlags2::from_raw(
        vk::AccessFlags2::MEMORY_READ.as_raw() | vk::AccessFlags2::MEMORY_WRITE.as_raw(),
    ));

    pub(crate) fn to_vk(self) -> vk::AccessFlags2 {
        self.0
    }
}

impl std::ops::BitOr for Access {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

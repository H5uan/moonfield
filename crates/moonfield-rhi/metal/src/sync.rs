//! Synchronization primitives.
//!
//! Metal's GPU-side ordering comes from queue submission order and the
//! device's shared-event timeline ([`Device::submit_and_wait`]): there is no
//! semaphore signal between image acquire and draw, because acquiring a
//! drawable blocks until it is ready and presenting happens after the
//! queue's committed work. [`Semaphore`] matches the Vulkan backend's
//! surface so the engine layer's frame code keeps one shape across
//! backends; on this backend it holds the device alive and nothing else.

use crate::device::{Device, DeviceContext};
use moonfield_rhi_core::Result;
use objc2_metal::MTLStages;

/// A pipeline stage mask for barriers, in the backend's own vocabulary
/// (mirrors the Vulkan backend's `Stage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stage(u64);

impl Stage {
    pub const VERTEX: Self = Self(1 << 0);
    pub const FRAGMENT: Self = Self(1 << 1);
    pub const COMPUTE: Self = Self(1 << 2);
    pub const TRANSFER: Self = Self(1 << 3);
    pub const DRAW_INDIRECT: Self = Self(1 << 4);
    pub const ALL_GRAPHICS: Self = Self(0b11);
    pub const ALL: Self = Self(0b11111);

    /// Map onto Metal 4's encoder-stage vocabulary (the barrier model is
    /// stage-to-stage; transfer/draw-indirect fold into the producer stage).
    pub(crate) fn to_metal(self) -> MTLStages {
        let mut stages = MTLStages::empty();
        if self.0 & Self::VERTEX.0 != 0 {
            stages |= MTLStages::Vertex;
        }
        if self.0 & Self::FRAGMENT.0 != 0 {
            stages |= MTLStages::Fragment;
        }
        if self.0 & Self::COMPUTE.0 != 0 {
            // Compute encoders carry the compute stages; render-side barriers
            // ignore this bit.
            stages |= MTLStages::Fragment;
        }
        stages
    }
}

impl std::ops::BitOr for Stage {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// An access mask for barriers (mirrors the Vulkan backend's `Access`).
/// Metal's barriers are stage-to-stage with no access granularity; the mask
/// exists so callers keep one barrier vocabulary across backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access(u64);

impl Access {
    pub const NONE: Self = Self(0);
    pub const SHADER_READ: Self = Self(1 << 0);
    pub const SHADER_WRITE: Self = Self(1 << 1);
    pub const COLOR_ATTACHMENT_READ: Self = Self(1 << 2);
    pub const COLOR_ATTACHMENT_WRITE: Self = Self(1 << 3);
    pub const MEMORY_READ: Self = Self(1 << 4);
    pub const MEMORY_WRITE: Self = Self(1 << 5);
}

impl std::ops::BitOr for Access {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// A synchronization token. See the module docs for the Metal sync model.
pub struct Semaphore {
    _ctx: DeviceContext,
}

impl Semaphore {
    /// Create a binary semaphore.
    pub fn new(device: &Device) -> Result<Self> {
        Ok(Self {
            _ctx: device.ctx().clone(),
        })
    }

    /// Create a timeline semaphore with an initial value. Timeline values
    /// live on the device's shared event on this backend; `initial` only
    /// records the caller's intent.
    pub fn new_timeline(device: &Device, initial: u64) -> Result<Self> {
        let _ = initial;
        Self::new(device)
    }
}

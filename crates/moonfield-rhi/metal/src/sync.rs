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

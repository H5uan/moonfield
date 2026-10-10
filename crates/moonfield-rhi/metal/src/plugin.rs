//! The shared device-level Metal singletons.

use std::sync::Arc;

use tracing::info;

use objc2_metal::MTLDevice;

use crate::{Device, Instance};
use moonfield_rhi_core::Result;

/// The shared device-level Metal singletons: one gated [`Instance`] and one
/// [`Device`] for the whole app. The engine layer's `RenderPlugin` inserts
/// this resource into the render world; headless one-shot consumers call
/// [`RenderDevice::new`] directly.
///
/// Mirrors the Vulkan backend's `RenderDevice` as a single resource.
/// Cloneable (cheap `Arc` clones) so windowed renderers can hold the device
/// alive independently of the resource's lifetime.
#[derive(Clone)]
pub struct RenderDevice {
    instance: Arc<Instance>,
    device: Arc<Device>,
}

impl RenderDevice {
    /// Create the shared instance + device (the Metal 4 family gate lives
    /// in [`Instance::new_headless`]).
    pub fn new() -> Result<Self> {
        let instance = Instance::new_headless()?;
        let device = Device::new(&instance)?;
        let name = device.ctx().shared().device().name();
        info!("Lunar Mare initialized Metal on device: {name}");
        // The Arcs give shared ownership, not cross-thread access: GPU
        // objects live on the main thread (the RHI's standing ownership
        // rule).
        #[allow(clippy::arc_with_non_send_sync)]
        Ok(Self {
            instance: Arc::new(instance),
            device: Arc::new(device),
        })
    }

    /// The gated Metal instance (system default device).
    pub fn instance(&self) -> &Arc<Instance> {
        &self.instance
    }

    /// The shared device (queue, residency set, timeline).
    pub fn device(&self) -> &Arc<Device> {
        &self.device
    }
}

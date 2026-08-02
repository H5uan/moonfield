//! wgpu device abstraction.

use crate::error::{Error, Result};

/// A wgpu instance, adapter, logical device, and queue bundle.
///
/// wgpu handles are cheap refcounted objects; cloning them is the idiomatic
/// way to share access, so the accessors hand out references and callers
/// clone as needed.
pub struct Device {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Device {
    /// Create a headless device: request an adapter with default options and
    /// a device with default limits (downlevel defaults, so this works on
    /// WebGPU and native drivers alike).
    ///
    /// Async because wgpu's request API is a future and there is no blocking
    /// executor available on wasm; native callers are expected to drive it
    /// from their own executor.
    pub async fn new_headless() -> Result<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .map_err(Error::from)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(Error::from)?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Access the wgpu device handle.
    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Access the wgpu queue handle.
    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Access the wgpu instance handle.
    pub fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// Access the wgpu adapter handle.
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }

    /// Escape hatch to the raw wgpu device handle.
    pub fn raw(&self) -> &wgpu::Device {
        &self.device
    }
}

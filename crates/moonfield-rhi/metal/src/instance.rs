//! Metal backend entry point: system default device discovery with the
//! Metal 4 family gate.

use moonfield_rhi_core::{Error, Result};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLCreateSystemDefaultDevice, MTLDevice, MTLGPUFamily};

/// Backend entry point. Metal has no instance concept; this type holds the
/// system default device and enforces the platform gate: the RHI targets
/// Metal 4 (`MTLGPUFamily.Metal4`), and anything older is an error rather
/// than a degraded path.
pub struct Instance {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
}

impl Instance {
    /// Acquire the system default device and verify it supports the Metal 4
    /// GPU family.
    pub fn new_headless() -> Result<Self> {
        let Some(device) = MTLCreateSystemDefaultDevice() else {
            return Err(Error::AdapterRequest("no Metal device available".into()));
        };
        if !device.supportsFamily(MTLGPUFamily::Metal4) {
            return Err(Error::Unsupported(
                "this device does not support Metal 4.0 or higher".into(),
            ));
        }
        Ok(Self { device })
    }

    /// The gated system default device.
    pub(crate) fn device(&self) -> &Retained<ProtocolObject<dyn MTLDevice>> {
        &self.device
    }
}

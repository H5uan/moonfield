//! Shared helpers for the facade contract tests.
//!
//! Every helper here goes through the facade's public API only; the selected
//! backend is whichever feature the build carries.

use moonfield_rhi::{Device, Instance};

/// Create a headless device on the selected backend, or `None` (with a
/// printed reason) when this machine cannot provide one — no driver at all,
/// or a driver missing the extensions the RHI requires (e.g.
/// `VK_EXT_descriptor_heap` on Vulkan).
#[allow(dead_code)]
pub fn headless_device() -> Option<Device> {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            moonfield_rhi::testing::skip_gpu(format!("no headless instance available ({err})"));
            return None;
        }
    };
    let device = Device::new(&instance, None);
    match device {
        Ok(device) => Some(device),
        Err(err) => {
            moonfield_rhi::testing::skip_gpu(format!("device creation failed ({err})"));
            None
        }
    }
}

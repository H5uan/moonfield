//! Shared helpers for the Lunar Mare GPU integration tests.

use crate::Instance;
use ash::vk;

/// Whether GPU test skips are forbidden. CI's strict GPU job (the
/// `gpu-contract` workflow) sets `MOONFIELD_STRICT_GPU` on a qualified
/// runner: hardware that cannot run the contract tests must fail the job,
/// not count as a pass. Regular CI keeps the tolerant default.
fn strict_gpu() -> bool {
    std::env::var_os("MOONFIELD_STRICT_GPU").is_some()
}

/// Announce a skip. In strict mode a skip is a hard failure instead: the
/// qualified runner's environment regressed, and the job must go red.
pub fn skip(reason: impl std::fmt::Display) {
    skip_forced(strict_gpu(), reason);
}

fn skip_forced(strict: bool, reason: impl std::fmt::Display) {
    if strict {
        panic!("strict GPU mode (MOONFIELD_STRICT_GPU is set): {reason}");
    }
    eprintln!("skipping: {reason}");
}

/// `VK_EXT_descriptor_heap` is required unconditionally by the engine
/// (`crates/moonfield-render/src/vulkan/device.rs`); only recent NVIDIA
/// drivers implement it. CI runs on lavapipe or machines without a compatible
/// driver, so the GPU tests skip with an explicit reason instead of failing
/// through the generic device-creation error path — unless strict mode is on.
///
/// Mirrors the physical-device selection in `Device::new` so the probe checks
/// the same device the engine would pick.
pub fn skip_if_descriptor_heap_missing(instance: &Instance) -> bool {
    let selected = instance
        .enumerate_physical_devices()
        .ok()
        .and_then(|devices| {
            devices.into_iter().min_by_key(|pd| {
                let mut props = vk::PhysicalDeviceProperties2::default();
                instance.physical_device_properties2(*pd, &mut props);
                match props.properties.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                    _ => 2,
                }
            })
        });
    let Some(pd) = selected else {
        skip("no Vulkan physical device available");
        return true;
    };

    let supported = unsafe { instance.raw().enumerate_device_extension_properties(pd) }.is_ok_and(
        |extensions| {
            extensions
                .iter()
                .any(|ext| unsafe { std::ffi::CStr::from_ptr(ext.extension_name.as_ptr()) }
                    == ash::ext::descriptor_heap::NAME)
        },
    );

    if !supported {
        skip("VK_EXT_descriptor_heap is required by the RHI and not supported by this driver");
    }
    !supported
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strict GPU contract: a skip must fail the test, not pass it.
    #[test]
    #[should_panic(expected = "strict GPU mode")]
    fn strict_mode_turns_skips_into_failures() {
        skip_forced(true, "no Vulkan physical device available");
    }
}

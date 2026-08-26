//! Shared helpers for the Lunar Mare GPU integration tests.

use ash::vk;
use moonfield_rhi::Instance;

/// `VK_EXT_descriptor_heap` is required unconditionally by the engine
/// (`crates/moonfield-render/src/vulkan/device.rs`); only recent NVIDIA
/// drivers implement it. CI runs on lavapipe or machines without a compatible
/// driver, so the GPU tests skip with an explicit reason instead of failing
/// through the generic device-creation error path.
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
        eprintln!("skipping: no Vulkan physical device available");
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
        eprintln!(
            "skipping: VK_EXT_descriptor_heap is required by the RHI and not supported by this driver"
        );
    }
    !supported
}

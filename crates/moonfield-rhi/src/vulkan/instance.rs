//! Vulkan instance abstraction.

use crate::error::{Error, Result};
use crate::vulkan::device::Device;
use crate::vulkan::swapchain::Surface;
use ash::vk;
use std::ffi::{CStr, c_char};
use std::sync::Arc;

/// The teardown-critical instance state, shared by `Arc`.
///
/// Every `Device` ([`DeviceShared`](crate::vulkan::device::DeviceShared)) and
/// every `Surface` holds an `Arc<InstanceShared>`, so the Vulkan instance
/// outlives everything created from it by construction — no device or surface
/// can ever reference a destroyed instance, and the instance is destroyed
/// exactly when the last of them goes away.
pub(crate) struct InstanceShared {
    entry: ash::Entry,
    instance: ash::Instance,
    surface_instance: ash::khr::surface::Instance,
}

impl Drop for InstanceShared {
    fn drop(&mut self) {
        // SAFETY: devices and surfaces created from this instance keep it
        // alive through their own `Arc<InstanceShared>`, so this drop runs
        // only after the last of them was destroyed; the destroy happens
        // exactly once, here.
        unsafe {
            self.instance.destroy_instance(None);
        }
    }
}

/// Vulkan instance and entry point.
pub struct Instance {
    shared: Arc<InstanceShared>,
}

impl Instance {
    /// Create a Vulkan instance with the requested extensions.
    ///
    /// `required_extensions` should contain platform surface extensions such as
    /// `VK_KHR_surface` and the platform-specific `VK_KHR_win32_surface`, etc.
    pub fn new(required_extensions: &[&CStr]) -> Result<Self> {
        // SAFETY: loading the Vulkan loader at runtime is always sound; a
        // missing driver is reported as an error, not UB.
        let entry = unsafe { ash::Entry::load() }
            .map_err(|e| Error::Backend(format!("failed to load Vulkan: {e}")))?;

        let app_name = std::ffi::CString::new("moonfield").unwrap();
        let engine_name = std::ffi::CString::new("Lunar Mare").unwrap();

        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_4);

        let extensions: Vec<*const c_char> =
            required_extensions.iter().map(|ext| ext.as_ptr()).collect();

        // Debug seam: the `validation` Cargo feature enables the Khronos
        // validation layer (needs the Vulkan SDK installed at runtime).
        #[cfg(feature = "validation")]
        let layers: Vec<*const c_char> = vec![c"VK_LAYER_KHRONOS_validation".as_ptr()];
        #[cfg(not(feature = "validation"))]
        let layers: Vec<*const c_char> = Vec::new();

        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extensions)
            .enabled_layer_names(&layers);

        // SAFETY: the entry is valid and the create info references
        // NUL-terminated extension/layer names and an application info that
        // all outlive the call.
        let instance = unsafe { entry.create_instance(&create_info, None) }
            .map_err(|e| Error::Backend(format!("failed to create Vulkan instance: {:?}", e)))?;

        let surface_instance = ash::khr::surface::Instance::load(&entry, &instance);

        Ok(Self {
            shared: Arc::new(InstanceShared {
                entry,
                instance,
                surface_instance,
            }),
        })
    }

    /// Create a headless-friendly instance with no surface extensions.
    pub fn new_headless() -> Result<Self> {
        Self::new(&[])
    }

    /// The shared instance state, for `Device`/`Surface` keepalive.
    /// Crate-internal.
    pub(crate) fn shared(&self) -> Arc<InstanceShared> {
        self.shared.clone()
    }

    /// Access the `ash::Entry` (needed e.g. for surface creation).
    pub(crate) fn entry(&self) -> &ash::Entry {
        &self.shared.entry
    }

    /// Access the raw `ash::Instance`.
    pub(crate) fn raw(&self) -> &ash::Instance {
        &self.shared.instance
    }

    /// Enumerate available physical devices.
    pub(crate) fn enumerate_physical_devices(&self) -> Result<Vec<vk::PhysicalDevice>> {
        // SAFETY: the instance is valid; enumeration returns handles owned by
        // the instance.
        unsafe {
            self.shared
                .instance
                .enumerate_physical_devices()
                .map_err(|e| {
                    Error::Backend(format!("failed to enumerate physical devices: {:?}", e))
                })
        }
    }

    /// Get properties for a physical device (Vulkan 1.1+ "2" query).
    ///
    /// The caller provides the output struct and may chain extended property
    /// structures (e.g. `PhysicalDeviceVulkan13Properties`) through its pNext
    /// pointer; the driver fills everything connected to the chain.
    pub(crate) fn physical_device_properties2(
        &self,
        device: vk::PhysicalDevice,
        out: &mut vk::PhysicalDeviceProperties2,
    ) {
        // SAFETY: the instance and physical device are valid, and `out` (with
        // its caller-chained sType list) is a writable struct the driver fills.
        unsafe {
            self.shared
                .instance
                .get_physical_device_properties2(device, out)
        }
    }

    /// Get features for a physical device (Vulkan 1.1+ "2" query).
    ///
    /// The caller provides the output struct and may chain extended feature
    /// structures through its pNext pointer; the driver fills everything
    /// connected to the chain.
    pub(crate) fn physical_device_features2(
        &self,
        device: vk::PhysicalDevice,
        out: &mut vk::PhysicalDeviceFeatures2,
    ) {
        // SAFETY: the instance and physical device are valid, and `out` (with
        // its caller-chained sType list) is a writable struct the driver fills.
        unsafe {
            self.shared
                .instance
                .get_physical_device_features2(device, out)
        }
    }

    /// Get queue family properties for a physical device (Vulkan 1.1+ "2"
    /// query); each entry's base data is in the `.queue_family_properties`
    /// field and extended structures can be attached through pNext.
    pub(crate) fn queue_family_properties2(
        &self,
        device: vk::PhysicalDevice,
    ) -> Vec<vk::QueueFamilyProperties2<'_>> {
        // SAFETY: the instance and physical device are valid; the query only
        // returns a count.
        let count = unsafe {
            self.shared
                .instance
                .get_physical_device_queue_family_properties2_len(device)
        };
        let mut out = vec![vk::QueueFamilyProperties2::default(); count];
        // SAFETY: `out` holds exactly `count` default-initialized entries
        // (valid sTypes) for the driver to fill, matching the `_len` query.
        unsafe {
            self.shared
                .instance
                .get_physical_device_queue_family_properties2(device, &mut out);
        }
        out
    }

    /// Check whether a queue family supports presentation to the given surface.
    pub(crate) fn get_physical_device_surface_support(
        &self,
        device: vk::PhysicalDevice,
        queue_family_index: u32,
        surface: vk::SurfaceKHR,
    ) -> bool {
        // SAFETY: the device and surface handles belong to this instance and
        // are live for the call (the surface is caller-owned).
        unsafe {
            self.shared
                .surface_instance
                .get_physical_device_surface_support(device, queue_family_index, surface)
                .unwrap_or(false)
        }
    }

    /// Whether the device's graphics queue family can present to `surface`.
    ///
    /// The shared device is created without a surface, so presentation
    /// support is validated per window surface at surface-creation time.
    pub fn supports_present(&self, device: &Device, surface: &Surface) -> bool {
        self.get_physical_device_surface_support(
            device.physical_device(),
            device.queue_family_indices().graphics,
            surface.raw(),
        )
    }
}

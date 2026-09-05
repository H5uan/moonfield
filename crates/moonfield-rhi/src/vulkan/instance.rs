//! Vulkan instance abstraction.

use crate::error::{Error, Result};
use crate::vulkan::device::Device;
use crate::vulkan::swapchain::Surface;
use ash::vk;
use std::ffi::{CStr, c_char};

/// Vulkan instance and entry point.
pub struct Instance {
    entry: ash::Entry,
    instance: ash::Instance,
    surface_instance: ash::khr::surface::Instance,
    /// Live logical devices created from this instance, shared with each
    /// `Device` (which holds its own `Arc` to the counter). Destroying an
    /// instance with live devices is invalid, so `Drop` leaks instead when
    /// this is non-zero (a teardown order where a `Device` outlives its
    /// `Instance` referent).
    live_devices: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Instance {
    /// Create a Vulkan instance with the requested extensions.
    ///
    /// `required_extensions` should contain platform surface extensions such as
    /// `VK_KHR_surface` and the platform-specific `VK_KHR_win32_surface`, etc.
    pub fn new(required_extensions: &[&CStr]) -> Result<Self> {
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

        let instance = unsafe { entry.create_instance(&create_info, None) }
            .map_err(|e| Error::Backend(format!("failed to create Vulkan instance: {:?}", e)))?;

        let surface_instance = ash::khr::surface::Instance::load(&entry, &instance);

        Ok(Self {
            entry,
            instance,
            surface_instance,
            live_devices: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    /// Create a headless-friendly instance with no surface extensions.
    pub fn new_headless() -> Result<Self> {
        Self::new(&[])
    }

    /// The live-device counter, shared with every `Device` created from
    /// this instance. Crate-internal: `Device` clones it at construction
    /// and decrements it exactly when it destroys its handle.
    pub(crate) fn live_devices(&self) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
        self.live_devices.clone()
    }

    /// Access the `ash::Entry` (needed e.g. for surface creation).
    pub(crate) fn entry(&self) -> &ash::Entry {
        &self.entry
    }

    /// Access the raw `ash::Instance`.
    pub(crate) fn raw(&self) -> &ash::Instance {
        &self.instance
    }

    /// Enumerate available physical devices.
    pub(crate) fn enumerate_physical_devices(&self) -> Result<Vec<vk::PhysicalDevice>> {
        unsafe {
            self.instance.enumerate_physical_devices().map_err(|e| {
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
        unsafe { self.instance.get_physical_device_properties2(device, out) }
    }

    /// Get queue family properties for a physical device (Vulkan 1.1+ "2"
    /// query); each entry's base data is in the `.queue_family_properties`
    /// field and extended structures can be attached through pNext.
    pub(crate) fn queue_family_properties2(
        &self,
        device: vk::PhysicalDevice,
    ) -> Vec<vk::QueueFamilyProperties2<'_>> {
        let count = unsafe {
            self.instance
                .get_physical_device_queue_family_properties2_len(device)
        };
        let mut out = vec![vk::QueueFamilyProperties2::default(); count];
        unsafe {
            self.instance
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
        unsafe {
            self.surface_instance
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

impl Drop for Instance {
    fn drop(&mut self) {
        if self.live_devices.load(std::sync::atomic::Ordering::Acquire) != 0 {
            // Destroying an instance with live devices is invalid — skip
            // the destroy and let the handle leak (a device leaked by
            // `Device::drop`'s guard against out-of-order teardown keeps
            // its count registered). `ash::Instance`'s own drop is a plain
            // handle release; `vkDestroyInstance` is only ever this call.
            tracing::error!(
                "instance dropped while logical devices are still alive; \
                 leaking the instance"
            );
            return;
        }
        unsafe {
            self.instance.destroy_instance(None);
        }
    }
}

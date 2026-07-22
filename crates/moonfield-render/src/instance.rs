//! Vulkan instance abstraction.

use crate::error::{Error, Result};
use ash::vk;
use std::ffi::{c_char, CStr};

/// Vulkan instance and entry point.
pub struct Instance {
    _entry: ash::Entry,
    instance: ash::Instance,
    surface_instance: ash::khr::surface::Instance,
}

impl Instance {
    /// Create a Vulkan instance with the requested extensions.
    ///
    /// `required_extensions` should contain platform surface extensions such as
    /// `VK_KHR_surface` and the platform-specific `VK_KHR_win32_surface`, etc.
    pub fn new(required_extensions: &[&CStr]) -> Result<Self> {
        let entry = unsafe { ash::Entry::load() }?;

        let app_name = std::ffi::CString::new("moonfield").unwrap();
        let engine_name = std::ffi::CString::new("Lunar Mare").unwrap();

        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&engine_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_3);

        let extensions: Vec<*const c_char> =
            required_extensions.iter().map(|ext| ext.as_ptr()).collect();

        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extensions);

        let instance = unsafe { entry.create_instance(&create_info, None) }
            .map_err(|e| Error::Backend(format!("failed to create Vulkan instance: {:?}", e)))?;

        let surface_instance = ash::khr::surface::Instance::new(&entry, &instance);

        Ok(Self {
            _entry: entry,
            instance,
            surface_instance,
        })
    }

    /// Create a headless-friendly instance with no surface extensions.
    pub fn new_headless() -> Result<Self> {
        Self::new(&[])
    }

    /// Access the raw `ash::Instance`.
    pub fn raw(&self) -> &ash::Instance {
        &self.instance
    }

    /// Access the `khr::surface` instance extension loader.
    pub fn surface_instance(&self) -> &ash::khr::surface::Instance {
        &self.surface_instance
    }

    /// Enumerate available physical devices.
    pub fn enumerate_physical_devices(&self) -> Result<Vec<vk::PhysicalDevice>> {
        unsafe {
            self.instance.enumerate_physical_devices().map_err(|e| {
                Error::Backend(format!("failed to enumerate physical devices: {:?}", e))
            })
        }
    }

    /// Get properties for a physical device.
    pub fn physical_device_properties(
        &self,
        device: vk::PhysicalDevice,
    ) -> vk::PhysicalDeviceProperties {
        unsafe { self.instance.get_physical_device_properties(device) }
    }

    /// Get queue family properties for a physical device.
    pub fn queue_family_properties(
        &self,
        device: vk::PhysicalDevice,
    ) -> Vec<vk::QueueFamilyProperties> {
        unsafe {
            self.instance
                .get_physical_device_queue_family_properties(device)
        }
    }

    /// Check whether a queue family supports presentation to the given surface.
    pub fn get_physical_device_surface_support(
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
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            self.instance.destroy_instance(None);
        }
    }
}

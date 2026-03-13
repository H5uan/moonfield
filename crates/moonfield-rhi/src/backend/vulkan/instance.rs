use std::sync::Arc;

use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use super::{VulkanAdapter, VulkanSurface};
use crate::{types::*, *};

pub struct VulkanInstance {
    pub entry: ash::Entry,
    pub instance: ash::Instance,
}

impl VulkanInstance {
    pub fn new() -> Result<Self, RhiError> {
        tracing::debug!("Creating Vulkan instance");
        Self::new_with_display(raw_window_handle::RawDisplayHandle::Windows(
            raw_window_handle::WindowsDisplayHandle::new(),
        ))
    }

    pub fn new_with_display(
        display: raw_window_handle::RawDisplayHandle,
    ) -> Result<Self, RhiError> {
        tracing::debug!("Creating Vulkan instance with display");
        unsafe {
            // load vulkan function pointers
            let entry = ash::Entry::load().map_err(|e| {
                tracing::error!("Failed to load Vulkan entry: {}", e);
                RhiError::InitializationFailed(format!(
                    "Failed to load Vulkan entry: {}",
                    e
                ))
            })?;

            let app_name = std::ffi::CString::new("Moonfield").unwrap();
            let engine_name =
                std::ffi::CString::new("MoonfieldEngine").unwrap();

            let app_info = ash::vk::ApplicationInfo::default()
                .application_name(&app_name)
                .application_version(ash::vk::make_api_version(0, 1, 4, 0))
                .engine_name(&engine_name)
                .engine_version(ash::vk::make_api_version(0, 1, 4, 0))
                .api_version(ash::vk::make_api_version(0, 1, 4, 0));

            let mut extension_names =
                ash_window::enumerate_required_extensions(display)
                    .map_err(|e| {
                        tracing::error!(
                            "Failed to enumerate required extensions: {}",
                            e
                        );
                        RhiError::InitializationFailed(format!(
                            "Failed to enumerate required extensions: {}",
                            e
                        ))
                    })?
                    .to_vec();

            // On macOS with MoltenVK, portability enumeration must be enabled explicitly
            #[cfg(target_os = "macos")]
            {
                extension_names.push(
                    b"VK_KHR_portability_enumeration\0".as_ptr() as *const i8,
                );
            }

            let layer_names = vec![
                std::ffi::CString::new("VK_LAYER_KHRONOS_validation").unwrap(),
            ];
            let layer_names_raw: Vec<*const i8> =
                layer_names.iter().map(|name| name.as_ptr()).collect();

            let mut create_info = ash::vk::InstanceCreateInfo::default()
                .application_info(&app_info)
                .enabled_extension_names(&extension_names)
                .enabled_layer_names(&layer_names_raw);

            // Required by Vulkan-Loader when only portability drivers (like MoltenVK) are present
            #[cfg(target_os = "macos")]
            {
                create_info.flags |=
                    vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
            }

            let instance =
                entry.create_instance(&create_info, None).map_err(|e| {
                    tracing::error!("Failed to create Vulkan instance: {}", e);
                    RhiError::InitializationFailed(format!(
                        "Failed to create Vulkan instance: {}",
                        e
                    ))
                })?;

            tracing::info!("Vulkan instance created successfully");
            Ok(Self { entry, instance })
        }
    }
}

impl Instance for VulkanInstance {
    fn create_surface(
        &self, window: &winit::window::Window,
    ) -> Result<Arc<dyn Surface>, RhiError> {
        tracing::debug!("Creating Vulkan surface for window");
        unsafe {
            let surface = ash_window::create_surface(
                &self.entry,
                &self.instance,
                window.display_handle().unwrap().as_raw(),
                window.window_handle().unwrap().as_raw(),
                None,
            )
            .map_err(|e| {
                tracing::error!("Failed to create Vulkan surface: {}", e);
                RhiError::InitializationFailed(format!(
                    "Failed to create Vulkan surface: {}",
                    e
                ))
            })?;

            let surface_loader =
                ash::khr::surface::Instance::new(&self.entry, &self.instance);

            tracing::debug!("Vulkan surface created successfully");
            Ok(Arc::new(VulkanSurface { surface, surface_loader }))
        }
    }

    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>> {
        tracing::debug!("Enumerating Vulkan physical devices");
        unsafe {
            let physical_devices =
                self.instance.enumerate_physical_devices().unwrap_or_default();

            let adapters: Vec<Arc<dyn Adapter>> = physical_devices
                .into_iter()
                .map(|pdevice| {
                    tracing::debug!("Found Vulkan physical device");
                    Arc::new(VulkanAdapter {
                        instance: self.instance.clone(),
                        physical_device: pdevice,
                    }) as Arc<dyn Adapter>
                })
                .collect();

            tracing::info!("Found {} Vulkan adapters", adapters.len());
            adapters
        }
    }
}

impl Drop for VulkanInstance {
    fn drop(&mut self) {
        unsafe {
            self.instance.destroy_instance(None);
        }
    }
}

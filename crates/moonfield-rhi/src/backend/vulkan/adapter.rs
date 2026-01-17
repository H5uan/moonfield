use crate::{types::*, *};
use ash::vk::Handle;
use std::sync::Arc;

use super::{VulkanDevice};

pub struct VulkanAdapter {
    pub instance: ash::Instance,
    pub physical_device: ash::vk::PhysicalDevice,
}

impl Adapter for VulkanAdapter {
    fn request_device(&self) -> Result<Arc<dyn Device>, RhiError> {
        tracing::debug!("Requesting Vulkan logical device");
        unsafe {
            let queue_family_properties = self
                .instance
                .get_physical_device_queue_family_properties(self.physical_device);

            let queue_family_index = queue_family_properties
                .iter()
                .enumerate()
                .find(|(_, props)| props.queue_flags.contains(ash::vk::QueueFlags::GRAPHICS))
                .map(|(i, _)| i as u32)
                .ok_or_else(|| {
                    tracing::error!("No suitable graphics queue family found");
                    RhiError::DeviceCreationFailed("No suitable graphics queue family found".to_string())
                })?;

            tracing::debug!("Found graphics queue family at index: {}", queue_family_index);

            let queue_priorities = [1.0];
            let queue_create_info = ash::vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family_index)
                .queue_priorities(&queue_priorities);

            let device_extension_names = [ash::khr::swapchain::NAME.as_ptr()];

            let mut features13 = ash::vk::PhysicalDeviceVulkan13Features::default()
                .dynamic_rendering(true)
                .synchronization2(true);

            let device_create_info = ash::vk::DeviceCreateInfo::default()
                .queue_create_infos(std::slice::from_ref(&queue_create_info))
                .enabled_extension_names(&device_extension_names)
                .push_next(&mut features13);

            let device = self
                .instance
                .create_device(self.physical_device, &device_create_info, None)
                .map_err(|e| {
                    tracing::error!("Failed to create logical device: {}", e);
                    RhiError::DeviceCreationFailed(format!("Failed to create logical device: {}", e))
                })?;

            let queue = device.get_device_queue(queue_family_index, 0);

            tracing::info!("Vulkan logical device created successfully");
            Ok(Arc::new(VulkanDevice {
                instance: self.instance.clone(),
                physical_device: self.physical_device,
                device,
                queue_family_index,
                queue,
            }))
        }
    }

    fn get_properties(&self) -> AdapterProperties {
        unsafe {
            let props = self.instance.get_physical_device_properties(self.physical_device);
            
            AdapterProperties {
                name: std::ffi::CStr::from_ptr(props.device_name.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
                vendor_id: props.vendor_id,
                device_id: props.device_id,
            }
        }
    }
}
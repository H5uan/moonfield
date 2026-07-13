//! Vulkan logical device abstraction.

use crate::error::{Error, Result};
use crate::instance::Instance;
use ash::vk;
use std::ffi::CStr;

const DEVICE_EXTENSIONS: &[&CStr] = &[ash::khr::swapchain::NAME];

/// Queue family indices selected for graphics and presentation.
#[derive(Debug, Clone, Copy)]
pub struct QueueFamilyIndices {
    pub graphics: u32,
    pub present: u32,
}

impl QueueFamilyIndices {
    /// Find suitable queue families for a physical device.
    ///
    /// If `surface` is `None`, presentation support is not checked and
    /// `present` is set to the graphics index.
    pub fn find(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Self> {
        let properties = instance.queue_family_properties(physical_device);

        let mut graphics = None;
        let mut present = None;

        for (index, props) in properties.iter().enumerate() {
            let index = index as u32;

            if graphics.is_none() && props.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                graphics = Some(index);
            }

            if let Some(surface) = surface {
                if present.is_none()
                    && instance.get_physical_device_surface_support(physical_device, index, surface)
                {
                    present = Some(index);
                }
            }
        }

        let graphics = graphics.ok_or(Error::Unsupported)?;
        let present = present.unwrap_or(graphics);

        Ok(Self { graphics, present })
    }

    /// Returns the unique queue family indices needed to create the device.
    pub fn unique_indices(&self) -> Vec<u32> {
        if self.graphics == self.present {
            vec![self.graphics]
        } else {
            vec![self.graphics, self.present]
        }
    }
}

/// Vulkan logical device and its primary queues.
pub struct Device {
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family_indices: QueueFamilyIndices,
}

impl Device {
    /// Create a logical device for the first suitable physical device.
    ///
    /// If `surface` is provided, presentation support is required.
    pub fn new(instance: &Instance, surface: Option<vk::SurfaceKHR>) -> Result<Self> {
        let physical_devices = instance.enumerate_physical_devices()?;
        if physical_devices.is_empty() {
            return Err(Error::Backend("no Vulkan-capable physical devices found".to_string()));
        }

        // Prefer discrete GPU, then integrated, then any.
        let physical_device = physical_devices
            .iter()
            .copied()
            .min_by_key(|pd| match instance.physical_device_properties(*pd).device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                _ => 2,
            })
            .ok_or(Error::Unsupported)?;

        Self::from_physical_device(instance, physical_device, surface)
    }

    /// Create a logical device from a specific physical device.
    pub fn from_physical_device(
        instance: &Instance,
        physical_device: vk::PhysicalDevice,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Self> {
        let queue_family_indices = QueueFamilyIndices::find(instance, physical_device, surface)?;

        let unique_indices = queue_family_indices.unique_indices();
        let queue_priorities = [1.0f32];
        let queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = unique_indices
            .iter()
            .map(|index| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(*index)
                    .queue_priorities(&queue_priorities)
            })
            .collect();

        let device_extension_names: Vec<*const i8> = DEVICE_EXTENSIONS
            .iter()
            .map(|name| name.as_ptr() as *const i8)
            .collect();

        let features = vk::PhysicalDeviceFeatures::default();

        let create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extension_names)
            .enabled_features(&features);

        let device = unsafe {
            instance
                .raw()
                .create_device(physical_device, &create_info, None)
        }
        .map_err(|e| Error::Backend(format!("failed to create logical device: {:?}", e)))?;

        let graphics_queue =
            unsafe { device.get_device_queue(queue_family_indices.graphics, 0) };
        let present_queue =
            unsafe { device.get_device_queue(queue_family_indices.present, 0) };

        Ok(Self {
            physical_device,
            device,
            graphics_queue,
            present_queue,
            queue_family_indices,
        })
    }

    /// Access the raw `ash::Device`.
    pub fn raw(&self) -> &ash::Device {
        &self.device
    }

    /// Access the underlying physical device handle.
    pub fn physical_device(&self) -> vk::PhysicalDevice {
        self.physical_device
    }

    /// Access the graphics queue.
    pub fn graphics_queue(&self) -> vk::Queue {
        self.graphics_queue
    }

    /// Access the presentation queue.
    pub fn present_queue(&self) -> vk::Queue {
        self.present_queue
    }

    /// Access the selected queue family indices.
    pub fn queue_family_indices(&self) -> QueueFamilyIndices {
        self.queue_family_indices
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_device(None);
        }
    }
}

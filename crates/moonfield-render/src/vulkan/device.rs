//! Vulkan logical device abstraction.

use crate::bindless;
use crate::error::{Error, Result};
use crate::vulkan::instance::Instance;
use ash::vk;
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use std::ffi::{c_char, CStr};
use std::sync::{Arc, Mutex};

const DEVICE_EXTENSIONS: &[&CStr] = &[ash::khr::swapchain::NAME];

/// Queue family indices selected for graphics and presentation.
#[derive(Debug, Clone, Copy)]
pub struct QueueFamilyIndices {
    pub graphics: u32,
    pub present: u32,
    pub compute: u32,
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
        let mut compute = None;

        for (index, props) in properties.iter().enumerate() {
            let index = index as u32;

            if graphics.is_none() && props.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                graphics = Some(index);
            }

            if compute.is_none()
                && props.queue_flags.contains(vk::QueueFlags::COMPUTE)
                && !props.queue_flags.contains(vk::QueueFlags::GRAPHICS)
            {
                compute = Some(index);
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
        let compute = compute.unwrap_or(graphics);

        Ok(Self {
            graphics,
            present,
            compute,
        })
    }

    /// Returns the unique queue family indices needed to create the device.
    pub fn unique_indices(&self) -> Vec<u32> {
        let mut indices = vec![self.graphics, self.present, self.compute];
        indices.sort_unstable();
        indices.dedup();
        indices
    }
}

/// Vulkan logical device and its primary queues.
pub struct Device {
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    graphics_queue: vk::Queue,
    compute_queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family_indices: QueueFamilyIndices,
    /// Shared GPU memory allocator for buffers and images. Wrapped in
    /// `Arc<Mutex>` so resources can hold clones and free their allocations
    /// on drop without a borrow on the device. `Option` so `Drop` can take it
    /// out and destroy it while the device handle is still valid.
    allocator: Option<Arc<Mutex<Allocator>>>,
}

impl Device {
    /// Create a logical device for the first suitable physical device.
    ///
    /// If `surface` is provided, presentation support is required.
    pub fn new(instance: &Instance, surface: Option<vk::SurfaceKHR>) -> Result<Self> {
        let physical_devices = instance.enumerate_physical_devices()?;
        if physical_devices.is_empty() {
            return Err(Error::Backend(
                "no Vulkan-capable physical devices found".to_string(),
            ));
        }

        // Prefer discrete GPU, then integrated, then any.
        let physical_device = physical_devices
            .iter()
            .copied()
            .min_by_key(
                |pd| match instance.physical_device_properties(*pd).device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                    _ => 2,
                },
            )
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

        let device_extension_names: Vec<*const c_char> =
            DEVICE_EXTENSIONS.iter().map(|name| name.as_ptr()).collect();

        let mut vulkan_12_features = vk::PhysicalDeviceVulkan12Features::default()
            .buffer_device_address(true)
            .timeline_semaphore(true)
            .descriptor_indexing(true)
            .descriptor_binding_sampled_image_update_after_bind(true)
            .descriptor_binding_partially_bound(true)
            .descriptor_binding_variable_descriptor_count(true)
            .runtime_descriptor_array(true)
            .shader_sampled_image_array_non_uniform_indexing(true);
        let mut vulkan_13_features =
            vk::PhysicalDeviceVulkan13Features::default().synchronization2(true);
        let mut features2 =
            vk::PhysicalDeviceFeatures2::default().features(vk::PhysicalDeviceFeatures::default());
        let _ = features2
            .push_next(&mut vulkan_12_features)
            .push_next(&mut vulkan_13_features);

        let create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&device_extension_names)
            .push_next(&mut features2);

        let device = unsafe {
            instance
                .raw()
                .create_device(physical_device, &create_info, None)
        }
        .map_err(|e| Error::Backend(format!("failed to create logical device: {:?}", e)))?;

        let graphics_queue = unsafe { device.get_device_queue(queue_family_indices.graphics, 0) };
        let compute_queue = unsafe { device.get_device_queue(queue_family_indices.compute, 0) };
        let present_queue = unsafe { device.get_device_queue(queue_family_indices.present, 0) };

        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance: instance.raw().clone(),
            device: device.clone(),
            physical_device,
            debug_settings: Default::default(),
            // Device enables `bufferDeviceAddress` (Vulkan 1.2 core) for the
            // bindless GPU pointer model; the allocator must match or
            // allocations cannot back a buffer device address.
            buffer_device_address: true,
            allocation_sizes: Default::default(),
        })
        .map_err(|e| Error::Backend(format!("failed to create GPU allocator: {e}")))?;

        Ok(Self {
            physical_device,
            device,
            graphics_queue,
            compute_queue,
            present_queue,
            queue_family_indices,
            allocator: Some(Arc::new(Mutex::new(allocator))),
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

    /// Access the compute queue.
    pub fn compute_queue(&self) -> vk::Queue {
        self.compute_queue
    }

    /// Access the presentation queue.
    pub fn present_queue(&self) -> vk::Queue {
        self.present_queue
    }

    pub fn queue(&self, ty: bindless::QueueType) -> vk::Queue {
        match ty {
            bindless::QueueType::Graphics => self.graphics_queue,
            bindless::QueueType::Compute => self.compute_queue,
        }
    }

    /// Access the selected queue family indices.
    pub fn queue_family_indices(&self) -> QueueFamilyIndices {
        self.queue_family_indices
    }

    /// Shared GPU memory allocator for buffers and images. Resources allocate
    /// through this and free their allocations on drop. Exposed so downstream
    /// code (e.g. the editor's egui backend) can share the same allocator.
    pub fn allocator(&self) -> &Arc<Mutex<Allocator>> {
        self.allocator
            .as_ref()
            .expect("allocator taken only during device drop")
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // The shared allocator's memory blocks must be freed while the logical
        // device is still alive (they call vkFreeMemory / vkUnmapMemory through
        // it), so the allocator is destroyed before vkDestroyDevice. Resources
        // (`Buffer`, images) drop before their owning device and release their
        // allocator `Arc`s, so by the time the device drops it is the last
        // referent and `try_unwrap` succeeds.
        if let Some(allocator) = self.allocator.take() {
            if let Ok(allocator) = Arc::try_unwrap(allocator) {
                drop(allocator);
            }
        }
        unsafe {
            self.device.destroy_device(None);
        }
    }
}

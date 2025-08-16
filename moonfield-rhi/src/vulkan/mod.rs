use std::{
    cell::{Cell, RefCell},
    ffi::CString,
    rc::{Rc, Weak},
};

use ash::khr::{surface, swapchain};
use ash::{Device as AshDevice, Entry, Instance, vk};
use winit::{
    event_loop::ActiveEventLoop,
    raw_window_handle::{HasDisplayHandle, HasWindowHandle},
    window::{Window, WindowAttributes},
};

use crate::{
    backend::{BackendCapabilities, Device, SharedGraphicsBackend},
    error::GraphicsError,
    geometry_buffer::{GeometryBufferDescriptor, GeometryBufferWarpper},
    vulkan::frame_buffer::VulkanFrameBuffer,
};

pub mod frame_buffer;

pub struct VulkanGraphicsBackend {
    entry: Entry,
    instance: Instance,
    device: AshDevice,
    physical_device: vk::PhysicalDevice,
    graphics_queue: vk::Queue,
    present_queue: vk::Queue,
    graphics_queue_family: u32,
    present_queue_family: u32,
    surface_loader: surface::Instance,
    surface: vk::SurfaceKHR,
    swapchain_loader: swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    swapchain_image_views: Vec<vk::ImageView>,
    swapchain_format: vk::Format,
    swapchain_extent: vk::Extent2D,
    command_pool: vk::CommandPool,
    pub named_objects: Cell<bool>,
    this: RefCell<Option<Weak<VulkanGraphicsBackend>>>,
}

impl VulkanGraphicsBackend {
    pub fn new(
        vsync: bool, _msaa_sample_count: Option<u8>,
        event_loop: &ActiveEventLoop, window_attrs: WindowAttributes,
        named_objects: bool,
    ) -> Result<(Window, SharedGraphicsBackend), GraphicsError> {
        // Create the window
        let window = event_loop
            .create_window(window_attrs)
            .map_err(|e| GraphicsError::WindowCreationError(e.to_string()))?;

        unsafe {
            let entry = Entry::load().map_err(|e| {
                GraphicsError::initialization_failed(
                    "Vulkan",
                    &format!("Failed to load Vulkan: {}", e),
                )
            })?;

            let instance = Self::create_instance(&entry, &window)?;

            // Create surface
            let surface = ash_window::create_surface(
                &entry,
                &instance,
                window.display_handle().unwrap().as_raw(),
                window.window_handle().unwrap().as_raw(),
                None,
            )
            .map_err(|e| {
                GraphicsError::device_error(
                    "Vulkan",
                    &format!("Failed to create surface: {}", e),
                )
            })?;

            let surface_loader = surface::Instance::new(&entry, &instance);

            // Pick physical device
            let physical_device = Self::pick_physical_device(
                &instance,
                &surface_loader,
                surface,
            )?;

            // Find queue families
            let (graphics_queue_family, present_queue_family) =
                Self::find_queue_families(
                    &instance,
                    physical_device,
                    &surface_loader,
                    surface,
                )?;

            // Create logical device
            let (device, graphics_queue, present_queue) =
                Self::create_logical_device(
                    &instance,
                    physical_device,
                    graphics_queue_family,
                    present_queue_family,
                )?;

            // Create swapchain
            let swapchain_loader = swapchain::Device::new(&instance, &device);
            let (
                swapchain,
                swapchain_format,
                swapchain_extent,
                swapchain_images,
            ) = Self::create_swapchain(
                &surface_loader,
                surface,
                physical_device,
                &swapchain_loader,
                &window,
                graphics_queue_family,
                present_queue_family,
                vsync,
            )?;

            // Create image views
            let swapchain_image_views = Self::create_image_views(
                &device,
                &swapchain_images,
                swapchain_format,
            )?;

            // Create command pool
            let command_pool =
                Self::create_command_pool(&device, graphics_queue_family)?;

            let backend = Self {
                entry,
                instance,
                device,
                physical_device,
                graphics_queue,
                present_queue,
                graphics_queue_family,
                present_queue_family,
                surface_loader,
                surface,
                swapchain_loader,
                swapchain,
                swapchain_images,
                swapchain_image_views,
                swapchain_format,
                swapchain_extent,
                command_pool,
                named_objects: Cell::new(named_objects),
                this: RefCell::new(None),
            };

            let shared_backend: SharedGraphicsBackend = Rc::new(backend);
            Ok((window, shared_backend))
        }
    }

    fn create_instance(
        entry: &Entry, window: &Window,
    ) -> Result<Instance, GraphicsError> {
        unsafe {
            let app_name = CString::new("Moonfield").unwrap();
            let engine_name = CString::new("Moonfield Engine").unwrap();

            let app_info = vk::ApplicationInfo::default()
                .application_name(app_name.as_c_str())
                .application_version(vk::make_api_version(0, 0, 1, 0))
                .engine_name(engine_name.as_c_str())
                .engine_version(vk::make_api_version(0, 0, 1, 0))
                .api_version(vk::make_api_version(0, 1, 3, 0));

            // Get required extensions from ash-window
            let extension_names = ash_window::enumerate_required_extensions(
                window.display_handle().unwrap().as_raw(),
            )
            .map_err(|e| {
                GraphicsError::initialization_failed(
                    "Vulkan",
                    &format!("Failed to get required extensions: {}", e),
                )
            })?;

            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info)
                .enabled_extension_names(&extension_names);

            entry.create_instance(&create_info, None).map_err(|e| {
                GraphicsError::initialization_failed(
                    "Vulkan",
                    &format!("Failed to create instance: {}", e),
                )
            })
        }
    }

    fn pick_physical_device(
        instance: &Instance, surface_loader: &surface::Instance,
        surface: vk::SurfaceKHR,
    ) -> Result<vk::PhysicalDevice, GraphicsError> {
        unsafe {
            let physical_devices =
                instance.enumerate_physical_devices().map_err(|e| {
                    GraphicsError::device_error(
                        "Vulkan",
                        &format!("Failed to enumerate physical devices: {}", e),
                    )
                })?;

            for device in physical_devices {
                if Self::is_device_suitable(
                    instance,
                    device,
                    surface_loader,
                    surface,
                )? {
                    return Ok(device);
                }
            }

            Err(GraphicsError::device_error(
                "Vulkan",
                "No suitable physical device found",
            ))
        }
    }

    fn is_device_suitable(
        instance: &Instance, device: vk::PhysicalDevice,
        surface_loader: &surface::Instance, surface: vk::SurfaceKHR,
    ) -> Result<bool, GraphicsError> {
        unsafe {
            // Check queue families
            let queue_families = Self::find_queue_families(
                instance,
                device,
                surface_loader,
                surface,
            );
            if queue_families.is_err() {
                return Ok(false);
            }

            // Check device extensions
            let available_extensions = instance
                .enumerate_device_extension_properties(device)
                .map_err(|e| {
                    GraphicsError::device_error(
                        "Vulkan",
                        &format!(
                            "Failed to enumerate device extensions: {}",
                            e
                        ),
                    )
                })?;

            let required_extensions = [swapchain::NAME];
            for required in &required_extensions {
                let found = available_extensions.iter().any(|ext| {
                    let name =
                        std::ffi::CStr::from_ptr(ext.extension_name.as_ptr());
                    name == *required
                });
                if !found {
                    return Ok(false);
                }
            }

            // Check swapchain support
            let formats = surface_loader
                .get_physical_device_surface_formats(device, surface)
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to get surface formats: {}", e),
                    )
                })?;
            let present_modes = surface_loader
                .get_physical_device_surface_present_modes(device, surface)
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to get present modes: {}", e),
                    )
                })?;

            Ok(!formats.is_empty() && !present_modes.is_empty())
        }
    }

    fn find_queue_families(
        instance: &Instance, device: vk::PhysicalDevice,
        surface_loader: &surface::Instance, surface: vk::SurfaceKHR,
    ) -> Result<(u32, u32), GraphicsError> {
        unsafe {
            let queue_families =
                instance.get_physical_device_queue_family_properties(device);

            let mut graphics_family = None;
            let mut present_family = None;

            for (index, queue_family) in queue_families.iter().enumerate() {
                let index = index as u32;

                if queue_family.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                    graphics_family = Some(index);
                }

                let present_support = surface_loader
                    .get_physical_device_surface_support(device, index, surface)
                    .map_err(|e| {
                        GraphicsError::device_error(
                            "Vulkan",
                            &format!("Failed to check present support: {}", e),
                        )
                    })?;

                if present_support {
                    present_family = Some(index);
                }

                if graphics_family.is_some() && present_family.is_some() {
                    break;
                }
            }

            match (graphics_family, present_family) {
                (Some(graphics), Some(present)) => Ok((graphics, present)),
                _ => Err(GraphicsError::device_error(
                    "Vulkan",
                    "Failed to find suitable queue families",
                )),
            }
        }
    }

    fn create_logical_device(
        instance: &Instance, physical_device: vk::PhysicalDevice,
        graphics_queue_family: u32, present_queue_family: u32,
    ) -> Result<(AshDevice, vk::Queue, vk::Queue), GraphicsError> {
        unsafe {
            let queue_priorities = [1.0f32];
            let mut queue_create_infos = Vec::new();

            // Graphics queue
            queue_create_infos.push(
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(graphics_queue_family)
                    .queue_priorities(&queue_priorities),
            );

            // Present queue (if different from graphics)
            if graphics_queue_family != present_queue_family {
                queue_create_infos.push(
                    vk::DeviceQueueCreateInfo::default()
                        .queue_family_index(present_queue_family)
                        .queue_priorities(&queue_priorities),
                );
            }

            let device_extensions = [swapchain::NAME.as_ptr()];

            let device_create_info = vk::DeviceCreateInfo::default()
                .queue_create_infos(&queue_create_infos)
                .enabled_extension_names(&device_extensions);

            let device = instance
                .create_device(physical_device, &device_create_info, None)
                .map_err(|e| {
                    GraphicsError::device_error(
                        "Vulkan",
                        &format!("Failed to create logical device: {}", e),
                    )
                })?;

            let graphics_queue =
                device.get_device_queue(graphics_queue_family, 0);
            let present_queue =
                device.get_device_queue(present_queue_family, 0);

            Ok((device, graphics_queue, present_queue))
        }
    }

    fn create_swapchain(
        surface_loader: &surface::Instance, surface: vk::SurfaceKHR,
        physical_device: vk::PhysicalDevice,
        swapchain_loader: &swapchain::Device, window: &Window,
        graphics_queue_family: u32, present_queue_family: u32, vsync: bool,
    ) -> Result<
        (vk::SwapchainKHR, vk::Format, vk::Extent2D, Vec<vk::Image>),
        GraphicsError,
    > {
        unsafe {
            let surface_capabilities = surface_loader
                .get_physical_device_surface_capabilities(
                    physical_device,
                    surface,
                )
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to get surface capabilities: {}", e),
                    )
                })?;

            let surface_formats = surface_loader
                .get_physical_device_surface_formats(physical_device, surface)
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to get surface formats: {}", e),
                    )
                })?;

            let present_modes = surface_loader
                .get_physical_device_surface_present_modes(
                    physical_device,
                    surface,
                )
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to get present modes: {}", e),
                    )
                })?;

            // Choose surface format
            let surface_format = surface_formats
                .iter()
                .find(|format| {
                    format.format == vk::Format::B8G8R8A8_SRGB
                        && format.color_space
                            == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
                .unwrap_or(&surface_formats[0]);

            // Choose present mode
            let present_mode = if vsync {
                vk::PresentModeKHR::FIFO // V-Sync
            } else {
                present_modes
                    .iter()
                    .find(|&&mode| mode == vk::PresentModeKHR::MAILBOX)
                    .copied()
                    .unwrap_or(vk::PresentModeKHR::FIFO)
            };

            // Choose extent
            let extent =
                if surface_capabilities.current_extent.width != u32::MAX {
                    surface_capabilities.current_extent
                } else {
                    let window_size = window.inner_size();
                    vk::Extent2D {
                        width: window_size.width.clamp(
                            surface_capabilities.min_image_extent.width,
                            surface_capabilities.max_image_extent.width,
                        ),
                        height: window_size.height.clamp(
                            surface_capabilities.min_image_extent.height,
                            surface_capabilities.max_image_extent.height,
                        ),
                    }
                };

            let image_count = (surface_capabilities.min_image_count + 1).min(
                if surface_capabilities.max_image_count > 0 {
                    surface_capabilities.max_image_count
                } else {
                    u32::MAX
                },
            );

            let queue_family_indices =
                if graphics_queue_family != present_queue_family {
                    vec![graphics_queue_family, present_queue_family]
                } else {
                    vec![graphics_queue_family]
                };

            let (sharing_mode, queue_family_indices_slice) =
                if graphics_queue_family != present_queue_family {
                    (
                        vk::SharingMode::CONCURRENT,
                        queue_family_indices.as_slice(),
                    )
                } else {
                    (vk::SharingMode::EXCLUSIVE, [].as_slice())
                };

            let swapchain_create_info = vk::SwapchainCreateInfoKHR::default()
                .surface(surface)
                .min_image_count(image_count)
                .image_format(surface_format.format)
                .image_color_space(surface_format.color_space)
                .image_extent(extent)
                .image_array_layers(1)
                .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(sharing_mode)
                .queue_family_indices(queue_family_indices_slice)
                .pre_transform(surface_capabilities.current_transform)
                .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                .present_mode(present_mode)
                .clipped(true);

            let swapchain = swapchain_loader
                .create_swapchain(&swapchain_create_info, None)
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to create swapchain: {}", e),
                    )
                })?;

            let swapchain_images = swapchain_loader
                .get_swapchain_images(swapchain)
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to get swapchain images: {}", e),
                    )
                })?;

            Ok((swapchain, surface_format.format, extent, swapchain_images))
        }
    }

    fn create_image_views(
        device: &AshDevice, swapchain_images: &[vk::Image],
        swapchain_format: vk::Format,
    ) -> Result<Vec<vk::ImageView>, GraphicsError> {
        unsafe {
            swapchain_images
                .iter()
                .map(|&image| {
                    let create_info = vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(swapchain_format)
                        .components(vk::ComponentMapping {
                            r: vk::ComponentSwizzle::IDENTITY,
                            g: vk::ComponentSwizzle::IDENTITY,
                            b: vk::ComponentSwizzle::IDENTITY,
                            a: vk::ComponentSwizzle::IDENTITY,
                        })
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        });

                    device.create_image_view(&create_info, None).map_err(|e| {
                        GraphicsError::device_error(
                            "Vulkan",
                            &format!("Failed to create image view: {}", e),
                        )
                    })
                })
                .collect()
        }
    }

    fn create_command_pool(
        device: &AshDevice, graphics_queue_family: u32,
    ) -> Result<vk::CommandPool, GraphicsError> {
        unsafe {
            let create_info = vk::CommandPoolCreateInfo::default()
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                .queue_family_index(graphics_queue_family);

            device.create_command_pool(&create_info, None).map_err(|e| {
                GraphicsError::command_error(
                    "Vulkan",
                    &format!("Failed to create command pool: {}", e),
                )
            })
        }
    }
}

impl Device for VulkanGraphicsBackend {
    fn back_buffer(
        &self,
    ) -> Result<crate::frame_buffer::SharedFrameBuffer, GraphicsError> {
        unsafe {
            // Create synchronization objects
            let semaphore_create_info = vk::SemaphoreCreateInfo::default();
            let fence_create_info = vk::FenceCreateInfo::default()
                .flags(vk::FenceCreateFlags::SIGNALED);

            let image_available_semaphore = self
                .device
                .create_semaphore(&semaphore_create_info, None)
                .map_err(|e| {
                    GraphicsError::command_error(
                        "Vulkan",
                        &format!("Failed to create semaphore: {}", e),
                    )
                })?;

            let render_finished_semaphore = self
                .device
                .create_semaphore(&semaphore_create_info, None)
                .map_err(|e| {
                    GraphicsError::command_error(
                        "Vulkan",
                        &format!("Failed to create semaphore: {}", e),
                    )
                })?;

            let in_flight_fence = self
                .device
                .create_fence(&fence_create_info, None)
                .map_err(|e| {
                    GraphicsError::command_error(
                        "Vulkan",
                        &format!("Failed to create fence: {}", e),
                    )
                })?;

            // Wait for fence and reset it
            self.device
                .wait_for_fences(&[in_flight_fence], true, u64::MAX)
                .map_err(|e| {
                    GraphicsError::command_error(
                        "Vulkan",
                        &format!("Failed to wait for fence: {}", e),
                    )
                })?;

            self.device.reset_fences(&[in_flight_fence]).map_err(|e| {
                GraphicsError::command_error(
                    "Vulkan",
                    &format!("Failed to reset fence: {}", e),
                )
            })?;

            // Acquire next image
            let (image_index, _) = self
                .swapchain_loader
                .acquire_next_image(
                    self.swapchain,
                    u64::MAX,
                    image_available_semaphore,
                    vk::Fence::null(),
                )
                .map_err(|e| {
                    GraphicsError::swapchain_error(
                        "Vulkan",
                        &format!("Failed to acquire next image: {}", e),
                    )
                })?;

            // Allocate command buffer
            let command_buffer_allocate_info =
                vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1);

            let command_buffers = self
                .device
                .allocate_command_buffers(&command_buffer_allocate_info)
                .map_err(|e| {
                    GraphicsError::command_error(
                        "Vulkan",
                        &format!("Failed to allocate command buffer: {}", e),
                    )
                })?;

            let command_buffer = command_buffers[0];

            // Begin command buffer
            let begin_info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

            self.device
                .begin_command_buffer(command_buffer, &begin_info)
                .map_err(|e| {
                    GraphicsError::command_error(
                        "Vulkan",
                        &format!("Failed to begin command buffer: {}", e),
                    )
                })?;

            // Get the swapchain image for this frame
            let swapchain_image = self.swapchain_images[image_index as usize];

            // Transition image layout for clearing
            let barrier = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );

            // Clear the image with a default color (dark blue)
            let clear_color = vk::ClearColorValue {
                float32: [1.0, 0.0, 0.0, 1.0], // Dark blue background
            };

            let range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            };

            self.device.cmd_clear_color_image(
                command_buffer,
                swapchain_image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &clear_color,
                &[range],
            );

            // Transition image layout for presentation
            let barrier = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );

            let frame_buffer = VulkanFrameBuffer {
                device: Rc::new(self.device.clone()),
                command_buffer,
                image_index,
                swapchain_extent: self.swapchain_extent,
                graphics_queue: self.graphics_queue,
                swapchain_loader: Rc::new(self.swapchain_loader.clone()),
                swapchain: self.swapchain,
                image_available_semaphore,
                render_finished_semaphore,
                in_flight_fence,
                clear_color: [0.0, 0.0, 0.0, 1.0],
                command_buffer_begun: true,
            };

            Ok(Box::new(frame_buffer))
        }
    }

    fn swap_buffers(&self) -> Result<(), GraphicsError> {
        // In Vulkan, buffer swapping is handled in the frame buffer's Drop implementation
        Ok(())
    }

    fn set_frame_size(&self, new_size: (u32, u32)) {
        // In a full implementation, this would recreate the swapchain
        // For now, we'll just log the resize request
        tracing::debug!(
            "Vulkan backend resize requested: {}x{}",
            new_size.0,
            new_size.1
        );
    }

    fn capabilities(&self) -> BackendCapabilities {
        unsafe {
            let device_properties = self
                .instance
                .get_physical_device_properties(self.physical_device);
            let max_buffer_length =
                device_properties.limits.max_storage_buffer_range as usize;

            BackendCapabilities { max_buffer_length }
        }
    }

    fn create_geometry_buffer(
        &self, _desc: GeometryBufferDescriptor,
    ) -> Result<GeometryBufferWarpper, GraphicsError> {
        // TODO: Implement Vulkan geometry buffer creation
        // For now, this is a stub implementation
        Err(GraphicsError::BackendUnavailable)
    }
}

impl Drop for VulkanGraphicsBackend {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().unwrap();

            self.device.destroy_command_pool(self.command_pool, None);

            for &image_view in &self.swapchain_image_views {
                self.device.destroy_image_view(image_view, None);
            }

            self.swapchain_loader.destroy_swapchain(self.swapchain, None);
            self.surface_loader.destroy_surface(self.surface, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

use crate::{types::*, *};
use ash::vk::{self, Handle};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::any::Any;
use std::ffi::{CStr, CString};
use std::sync::Arc;

pub struct VulkanInstance {
    entry: ash::Entry,
    instance: ash::Instance,
}

impl VulkanInstance {
    pub fn new() -> Result<Self, RhiError> {
        tracing::debug!("Creating Vulkan instance");
        Self::new_with_display(raw_window_handle::RawDisplayHandle::Windows(
            raw_window_handle::WindowsDisplayHandle::new()
        ))
    }

    pub fn new_with_display(display: raw_window_handle::RawDisplayHandle) -> Result<Self, RhiError> {
        tracing::debug!("Creating Vulkan instance with display");
        unsafe {
            let entry = ash::Entry::load()
                .map_err(|e| {
                    tracing::error!("Failed to load Vulkan entry: {}", e);
                    RhiError::InitializationFailed(format!("Failed to load Vulkan entry: {}", e))
                })?;
            
            let app_name = CString::new("Moonfield").unwrap();
            let engine_name = CString::new("MoonfieldEngine").unwrap();
            
            let app_info = vk::ApplicationInfo::default()
                .application_name(&app_name)
                .application_version(vk::make_api_version(0, 1, 0, 0))
                .engine_name(&engine_name)
                .engine_version(vk::make_api_version(0, 1, 0, 0))
                .api_version(vk::API_VERSION_1_3);

            let extension_names = ash_window::enumerate_required_extensions(display)
            .map_err(|e| {
                tracing::error!("Failed to enumerate required extensions: {}", e);
                RhiError::InitializationFailed(format!("Failed to enumerate required extensions: {}", e))
            })?
            .to_vec();

            let layer_names = vec![CString::new("VK_LAYER_KHRONOS_validation").unwrap()];
            let layer_names_raw: Vec<*const i8> = layer_names
                .iter()
                .map(|name| name.as_ptr())
                .collect();

            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info)
                .enabled_extension_names(&extension_names)
                .enabled_layer_names(&layer_names_raw);

            let instance = entry
                .create_instance(&create_info, None)
                .map_err(|e| {
                    tracing::error!("Failed to create Vulkan instance: {}", e);
                    RhiError::InitializationFailed(format!("Failed to create Vulkan instance: {}", e))
                })?;

            tracing::info!("Vulkan instance created successfully");
            Ok(Self { entry, instance })
        }
    }
}

impl Instance for VulkanInstance {
    fn create_surface(&self, window: &winit::window::Window) -> Result<Arc<dyn Surface>, RhiError> {
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
                RhiError::InitializationFailed(format!("Failed to create Vulkan surface: {}", e))
            })?;

            let surface_loader = ash::khr::surface::Instance::new(&self.entry, &self.instance);

            tracing::debug!("Vulkan surface created successfully");
            Ok(Arc::new(VulkanSurface {
                surface,
                surface_loader,
            }))
        }
    }

    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>> {
        tracing::debug!("Enumerating Vulkan physical devices");
        unsafe {
            let physical_devices = self.instance.enumerate_physical_devices()
                .unwrap_or_default();
            
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

pub struct VulkanSurface {
    surface: vk::SurfaceKHR,
    surface_loader: ash::khr::surface::Instance,
}

impl Surface for VulkanSurface {
    fn get_capabilities(&self, adapter: &dyn Adapter) -> SurfaceCapabilities {
        let vk_adapter = (adapter as &dyn Any)
            .downcast_ref::<VulkanAdapter>()
            .expect("adapter must be VulkanAdapter");

        unsafe {
            let caps = self
                .surface_loader
                .get_physical_device_surface_capabilities(vk_adapter.physical_device, self.surface)
                .unwrap();

            let formats = self
                .surface_loader
                .get_physical_device_surface_formats(vk_adapter.physical_device, self.surface)
                .unwrap();

            SurfaceCapabilities {
                formats: formats.iter().map(|f| match f.format {
                    vk::Format::B8G8R8A8_UNORM => Format::B8G8R8A8Unorm,
                    vk::Format::B8G8R8A8_SRGB => Format::B8G8R8A8Srgb,
                    _ => Format::B8G8R8A8Unorm,
                }).collect(),
                present_modes: vec![PresentMode::Fifo],
                min_image_count: caps.min_image_count,
                max_image_count: caps.max_image_count,
            }
        }
    }
}

impl Drop for VulkanSurface {
    fn drop(&mut self) {
        unsafe {
            self.surface_loader.destroy_surface(self.surface, None);
        }
    }
}

pub struct VulkanAdapter {
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
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
                .find(|(_, props)| props.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .map(|(i, _)| i as u32)
                .ok_or_else(|| {
                    tracing::error!("No suitable graphics queue family found");
                    RhiError::DeviceCreationFailed("No suitable graphics queue family found".to_string())
                })?;

            tracing::debug!("Found graphics queue family at index: {}", queue_family_index);

            let queue_priorities = [1.0];
            let queue_create_info = vk::DeviceQueueCreateInfo::default()
                .queue_family_index(queue_family_index)
                .queue_priorities(&queue_priorities);

            let device_extension_names = [ash::khr::swapchain::NAME.as_ptr()];

            let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
                .dynamic_rendering(true)
                .synchronization2(true);

            let device_create_info = vk::DeviceCreateInfo::default()
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
                name: CStr::from_ptr(props.device_name.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
                vendor_id: props.vendor_id,
                device_id: props.device_id,
            }
        }
    }
}

pub struct VulkanDevice {
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue_family_index: u32,
    queue: vk::Queue,
}

impl Device for VulkanDevice {
    fn create_swapchain(&self, desc: &SwapchainDescriptor) -> Result<Arc<dyn Swapchain>, RhiError> {
        tracing::debug!("Creating Vulkan swapchain with format: {:?}, extent: {:?}", desc.format, desc.extent);
        let vk_surface = (&*desc.surface as &dyn Any)
            .downcast_ref::<VulkanSurface>()
            .expect("surface must be VulkanSurface");

        unsafe {
            let swapchain_loader = ash::khr::swapchain::Device::new(&self.instance, &self.device);

            let format = match desc.format {
                Format::B8G8R8A8Unorm => vk::Format::B8G8R8A8_UNORM,
                Format::B8G8R8A8Srgb => vk::Format::B8G8R8A8_SRGB,
                _ => vk::Format::B8G8R8A8_UNORM,
            };

            let create_info = vk::SwapchainCreateInfoKHR::default()
                .surface(vk_surface.surface)
                .min_image_count(desc.image_count)
                .image_format(format)
                .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                .image_extent(vk::Extent2D {
                    width: desc.extent.width,
                    height: desc.extent.height,
                })
                .image_array_layers(1)
                .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
                .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                .present_mode(vk::PresentModeKHR::FIFO)
                .clipped(true);

            let swapchain = swapchain_loader
                .create_swapchain(&create_info, None)
                .map_err(|e| {
                    tracing::error!("Failed to create swapchain: {}", e);
                    RhiError::SwapchainCreationFailed(format!("Failed to create swapchain: {}", e))
                })?;

            let images = swapchain_loader
                .get_swapchain_images(swapchain)
                .map_err(|e| {
                    tracing::error!("Failed to get swapchain images: {}", e);
                    RhiError::SwapchainCreationFailed(format!("Failed to get swapchain images: {}", e))
                })?;

            let image_views: Vec<_> = images
                .iter()
                .map(|&image| {
                    let create_info = vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .components(vk::ComponentMapping::default())
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        });

                    self.device.create_image_view(&create_info, None)
                        .map_err(|e| {
                            tracing::error!("Failed to create image view: {}", e);
                            e
                        })
                        .unwrap()
                })
                .collect();

            let semaphore_create_info = vk::SemaphoreCreateInfo::default();
            
            let mut image_available_semaphores = Vec::new();
            let mut render_finished_semaphores = Vec::new();
            
            for i in 0..images.len() {
                let image_available = self
                    .device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| {
                        tracing::error!("Failed to create image available semaphore {}: {}", i, e);
                        RhiError::SwapchainCreationFailed(format!("Failed to create image available semaphore: {}", e))
                    })?;
                image_available_semaphores.push(image_available);
                
                let render_finished = self
                    .device
                    .create_semaphore(&semaphore_create_info, None)
                    .map_err(|e| {
                        tracing::error!("Failed to create render finished semaphore {}: {}", i, e);
                        RhiError::SwapchainCreationFailed(format!("Failed to create render finished semaphore: {}", e))
                    })?;
                render_finished_semaphores.push(render_finished);
            }

            let image_count = images.len();
            tracing::info!("Vulkan swapchain created successfully with {} images", image_count);

            Ok(Arc::new(VulkanSwapchain {
                device: self.device.clone(),
                swapchain_loader,
                swapchain,
                images,
                image_views,
                format: desc.format,
                extent: desc.extent,
                image_available_semaphores,
                render_finished_semaphores,
                queue: self.queue,
                current_frame: std::sync::atomic::AtomicUsize::new(0),
                image_layouts: std::sync::Mutex::new(vec![vk::ImageLayout::UNDEFINED; image_count]),
            }))
        }
    }

    fn create_shader_module(&self, desc: &ShaderModuleDescriptor) -> Result<Arc<dyn ShaderModule>, RhiError> {
        tracing::debug!("Creating Vulkan shader module for stage: {:?}", desc.stage);
        unsafe {
            let code = std::slice::from_raw_parts(
                desc.code.as_ptr() as *const u32,
                desc.code.len() / 4,
            );

            let create_info = vk::ShaderModuleCreateInfo::default().code(code);

            let shader_module = self
                .device
                .create_shader_module(&create_info, None)
                .map_err(|e| {
                    tracing::error!("Failed to create shader module: {}", e);
                    RhiError::ShaderCompilationFailed(ShaderCompilationError::CompilationError(format!("Failed to create shader module: {}", e)))
                })?;

            tracing::info!("Vulkan shader module created successfully for stage: {:?}", desc.stage);
            Ok(Arc::new(VulkanShaderModule {
                device: self.device.clone(),
                shader_module,
                stage: desc.stage,
            }))
        }
    }

    fn create_pipeline(&self, desc: &GraphicsPipelineDescriptor) -> Result<Arc<dyn Pipeline>, RhiError> {
        unsafe {
            let vs = (&*desc.vertex_shader as &dyn Any)
                .downcast_ref::<VulkanShaderModule>()
                .expect("vertex_shader must be VulkanShaderModule");

            let fs = (&*desc.fragment_shader as &dyn Any)
                .downcast_ref::<VulkanShaderModule>()
                .expect("fragment_shader must be VulkanShaderModule");

            let entry_name = CString::new("main").unwrap();

            let shader_stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(vs.shader_module)
                    .name(&entry_name),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(fs.shader_module)
                    .name(&entry_name),
            ];

            let binding_descriptions: Vec<_> = desc.vertex_input.bindings.iter().map(|b| {
                vk::VertexInputBindingDescription {
                    binding: b.binding,
                    stride: b.stride,
                    input_rate: match b.input_rate {
                        VertexInputRate::Vertex => vk::VertexInputRate::VERTEX,
                        VertexInputRate::Instance => vk::VertexInputRate::INSTANCE,
                    },
                }
            }).collect();

            let attribute_descriptions: Vec<_> = desc.vertex_input.attributes.iter().map(|a| {
                vk::VertexInputAttributeDescription {
                    location: a.location,
                    binding: a.binding,
                    format: match a.format {
                        VertexFormat::Float32x2 => vk::Format::R32G32_SFLOAT,
                        VertexFormat::Float32x3 => vk::Format::R32G32B32_SFLOAT,
                        VertexFormat::Float32x4 => vk::Format::R32G32B32A32_SFLOAT,
                    },
                    offset: a.offset,
                }
            }).collect();

            let vertex_input_state = vk::PipelineVertexInputStateCreateInfo::default()
                .vertex_binding_descriptions(&binding_descriptions)
                .vertex_attribute_descriptions(&attribute_descriptions);

            let input_assembly_state = vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
                .primitive_restart_enable(false);

            let viewport_state = vk::PipelineViewportStateCreateInfo::default()
                .viewport_count(1)
                .scissor_count(1);

            let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
                .depth_clamp_enable(false)
                .rasterizer_discard_enable(false)
                .polygon_mode(vk::PolygonMode::FILL)
                .cull_mode(vk::CullModeFlags::BACK)
                .front_face(vk::FrontFace::CLOCKWISE)
                .depth_bias_enable(false)
                .line_width(1.0);

            let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(vk::SampleCountFlags::TYPE_1)
                .sample_shading_enable(false);

            let color_blend_attachment = vk::PipelineColorBlendAttachmentState::default()
                .blend_enable(false)
                .color_write_mask(vk::ColorComponentFlags::RGBA);

            let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
                .logic_op_enable(false)
                .attachments(std::slice::from_ref(&color_blend_attachment));

            let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
            let dynamic_state = vk::PipelineDynamicStateCreateInfo::default()
                .dynamic_states(&dynamic_states);

            let pipeline_layout_create_info = vk::PipelineLayoutCreateInfo::default();
            let pipeline_layout = self
                .device
                .create_pipeline_layout(&pipeline_layout_create_info, None)
                .map_err(|e| RhiError::PipelineCreationFailed(format!("Failed to create pipeline layout: {}", e)))?;

            let format = match desc.render_pass_format {
                Format::B8G8R8A8Unorm => vk::Format::B8G8R8A8_UNORM,
                Format::B8G8R8A8Srgb => vk::Format::B8G8R8A8_SRGB,
                _ => vk::Format::B8G8R8A8_UNORM,
            };

            let color_attachment_format = [format];
            let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
                .color_attachment_formats(&color_attachment_format);

            let pipeline_create_info = vk::GraphicsPipelineCreateInfo::default()
                .stages(&shader_stages)
                .vertex_input_state(&vertex_input_state)
                .input_assembly_state(&input_assembly_state)
                .viewport_state(&viewport_state)
                .rasterization_state(&rasterization_state)
                .multisample_state(&multisample_state)
                .color_blend_state(&color_blend_state)
                .dynamic_state(&dynamic_state)
                .layout(pipeline_layout)
                .push_next(&mut rendering_info);

            let pipelines = self
                .device
                .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_create_info], None)
                .map_err(|e| RhiError::PipelineCreationFailed(format!("Failed to create graphics pipeline: {:?}", e)))?;

            Ok(Arc::new(VulkanPipeline {
                device: self.device.clone(),
                pipeline: pipelines[0],
                pipeline_layout,
            }))
        }
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Result<Arc<dyn Buffer>, RhiError> {
        unsafe {
            let usage = match desc.usage {
                BufferUsage::Vertex => vk::BufferUsageFlags::VERTEX_BUFFER,
                BufferUsage::Index => vk::BufferUsageFlags::INDEX_BUFFER,
                BufferUsage::Uniform => vk::BufferUsageFlags::UNIFORM_BUFFER,
            };

            let buffer_info = vk::BufferCreateInfo::default()
                .size(desc.size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);

            let buffer = self
                .device
                .create_buffer(&buffer_info, None)
                .map_err(|e| RhiError::BufferCreationFailed(format!("Failed to create buffer: {}", e)))?;

            let mem_requirements = self.device.get_buffer_memory_requirements(buffer);

            let memory_type_index = self.find_memory_type(
                mem_requirements.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            ).ok_or_else(|| RhiError::BufferCreationFailed("Could not find suitable memory type for buffer".to_string()))?;

            let alloc_info = vk::MemoryAllocateInfo::default()
                .allocation_size(mem_requirements.size)
                .memory_type_index(memory_type_index);

            let memory = self
                .device
                .allocate_memory(&alloc_info, None)
                .map_err(|e| RhiError::BufferCreationFailed(format!("Failed to allocate buffer memory: {}", e)))?;

            self.device
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| RhiError::BufferCreationFailed(format!("Failed to bind buffer memory: {}", e)))?;

            Ok(Arc::new(VulkanBuffer {
                device: self.device.clone(),
                buffer,
                memory,
                size: desc.size,
            }))
        }
    }

    fn create_command_pool(&self, swapchain: &Arc<dyn Swapchain>) -> Result<Arc<dyn CommandPool>, RhiError> {
        unsafe {
            let pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(self.queue_family_index)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

            let command_pool = self
                .device
                .create_command_pool(&pool_info, None)
                .map_err(|e| RhiError::CommandPoolCreationFailed(format!("Failed to create command pool: {}", e)))?;

            let swapchain_weak = {
                let raw_ptr = Arc::as_ptr(swapchain);
                let vk_swapchain_ptr = raw_ptr as *const VulkanSwapchain;
                let temp_arc = Arc::from_raw(vk_swapchain_ptr);
                let weak = Arc::downgrade(&temp_arc);
                let _ = Arc::into_raw(temp_arc);
                weak
            };

            Ok(Arc::new(VulkanCommandPool {
                device: self.device.clone(),
                command_pool,
                swapchain: swapchain_weak,
            }))
        }
    }

    fn get_queue(&self) -> Arc<dyn Queue> {
        Arc::new(VulkanQueue {
            device: self.device.clone(),
            queue: self.queue,
        })
    }
}

impl VulkanDevice {
    unsafe fn find_memory_type(&self, type_filter: u32, properties: vk::MemoryPropertyFlags) -> Option<u32> {
        let mem_properties = unsafe {
            self.instance.get_physical_device_memory_properties(self.physical_device)
        };
        
        for i in 0..mem_properties.memory_type_count {
            if (type_filter & (1 << i)) != 0
                && mem_properties.memory_types[i as usize].property_flags.contains(properties)
            {
                return Some(i);
            }
        }
        None
    }
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_device(None);
        }
    }
}

pub struct VulkanSwapchain {
    device: ash::Device,
    swapchain_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    format: Format,
    extent: Extent2D,
    image_available_semaphores: Vec<vk::Semaphore>,
    render_finished_semaphores: Vec<vk::Semaphore>,
    queue: vk::Queue,
    current_frame: std::sync::atomic::AtomicUsize,
    image_layouts: std::sync::Mutex<Vec<vk::ImageLayout>>,
}

impl Swapchain for VulkanSwapchain {
    fn acquire_next_image(&self) -> Result<SwapchainImage, RhiError> {
        unsafe {
            let frame = self.current_frame.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let semaphore = self.image_available_semaphores[frame % self.image_available_semaphores.len()];
            
            let (index, _) = self
                .swapchain_loader
                .acquire_next_image(
                    self.swapchain,
                    u64::MAX,
                    semaphore,
                    vk::Fence::null(),
                )
                .map_err(|e| RhiError::AcquireImageFailed(format!("Failed to acquire next swapchain image: {}", e)))?;

            Ok(SwapchainImage {
                index,
                image_view: self.image_views[index as usize].as_raw() as usize,
                wait_semaphore: semaphore.as_raw(),
                signal_semaphore: self.render_finished_semaphores[index as usize].as_raw(),
            })
        }
    }

    fn present(&self, image: SwapchainImage) -> Result<(), RhiError> {
        unsafe {
            let swapchains = [self.swapchain];
            let image_indices = [image.index];
            let wait_semaphore = vk::Semaphore::from_raw(image.signal_semaphore);
            let wait_semaphores = [wait_semaphore];

            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&wait_semaphores)
                .swapchains(&swapchains)
                .image_indices(&image_indices);

            self.device.queue_wait_idle(self.queue).ok();

            self.swapchain_loader
                .queue_present(self.queue, &present_info)
                .map_err(|e| RhiError::PresentFailed(format!("Failed to present swapchain image: {}", e)))?;

            Ok(())
        }
    }

    fn get_format(&self) -> Format {
        self.format
    }

    fn get_extent(&self) -> Extent2D {
        self.extent
    }
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            for &image_view in &self.image_views {
                self.device.destroy_image_view(image_view, None);
            }
            for &semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &semaphore in &self.render_finished_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.swapchain_loader.destroy_swapchain(self.swapchain, None);
        }
    }
}

pub struct VulkanShaderModule {
    device: ash::Device,
    shader_module: vk::ShaderModule,
    stage: ShaderStage,
}

impl ShaderModule for VulkanShaderModule {}

impl Drop for VulkanShaderModule {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_shader_module(self.shader_module, None);
        }
    }
}

pub struct VulkanPipeline {
    device: ash::Device,
    pipeline: vk::Pipeline,
    pipeline_layout: vk::PipelineLayout,
}

impl Pipeline for VulkanPipeline {}

impl Drop for VulkanPipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
            self.device.destroy_pipeline_layout(self.pipeline_layout, None);
        }
    }
}

pub struct VulkanBuffer {
    device: ash::Device,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

impl Buffer for VulkanBuffer {
    fn map(&self) -> Result<*mut u8, RhiError> {
        unsafe {
            self.device
                .map_memory(self.memory, 0, self.size, vk::MemoryMapFlags::empty())
                .map(|ptr| ptr as *mut u8)
                .map_err(|e| RhiError::MapFailed(format!("Failed to map buffer memory: {}", e)))
        }
    }

    fn unmap(&self) {
        unsafe {
            self.device.unmap_memory(self.memory);
        }
    }
}

impl Drop for VulkanBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

pub struct VulkanCommandPool {
    device: ash::Device,
    command_pool: vk::CommandPool,
    swapchain: std::sync::Weak<VulkanSwapchain>,
}

impl CommandPool for VulkanCommandPool {
    fn allocate_command_buffer(&self) -> Result<Arc<dyn CommandBuffer>, RhiError> {
        unsafe {
            let alloc_info = vk::CommandBufferAllocateInfo::default()
                .command_pool(self.command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);

            let command_buffers = self
                .device
                .allocate_command_buffers(&alloc_info)
                .map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to allocate command buffers: {}", e)))?;

            Ok(Arc::new(VulkanCommandBuffer {
                device: self.device.clone(),
                command_buffer: command_buffers[0],
                swapchain: Some(self.swapchain.clone()),
                current_image_index: std::cell::Cell::new(None),
            }))
        }
    }
}

impl Drop for VulkanCommandPool {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            self.device.destroy_command_pool(self.command_pool, None);
        }
    }
}

pub struct VulkanCommandBuffer {
    device: ash::Device,
    command_buffer: vk::CommandBuffer,
    swapchain: Option<std::sync::Weak<VulkanSwapchain>>,
    current_image_index: std::cell::Cell<Option<u32>>,
}

impl CommandBuffer for VulkanCommandBuffer {
    fn begin(&self) -> Result<(), RhiError> {
        unsafe {
            let begin_info = vk::CommandBufferBeginInfo::default();
            self.device
                .begin_command_buffer(self.command_buffer, &begin_info)
                .map_err(|e| RhiError::InitializationFailed(format!("Failed to begin command buffer: {}", e)))
        }
    }

    fn end(&self) -> Result<(), RhiError> {
        unsafe {
            self.device
                .end_command_buffer(self.command_buffer)
                .map_err(|e| RhiError::InitializationFailed(format!("Failed to end command buffer: {}", e)))
        }
    }

    fn begin_render_pass(&self, desc: &RenderPassDescriptor, image: &SwapchainImage) {
        unsafe {
            let image_view = vk::ImageView::from_raw(image.image_view as u64);
            let swapchain = self.swapchain.as_ref().and_then(|w| w.upgrade()).unwrap();
            let swapchain_image = swapchain.images[image.index as usize];

            self.current_image_index.set(Some(image.index));

            let mut layouts = swapchain.image_layouts.lock().unwrap();
            let old_layout = layouts[image.index as usize];

            let barrier = vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
                .src_access_mask(vk::AccessFlags2::empty())
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(old_layout)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            let dependency_info = vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&barrier));

            self.device.cmd_pipeline_barrier2(self.command_buffer, &dependency_info);

            layouts[image.index as usize] = vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL;
            drop(layouts);

            let color_attachment = &desc.color_attachments[0];
            
            let clear_value = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: color_attachment.clear_value,
                },
            };

            let rendering_attachment = vk::RenderingAttachmentInfo::default()
                .image_view(image_view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(match color_attachment.load_op {
                    LoadOp::Load => vk::AttachmentLoadOp::LOAD,
                    LoadOp::Clear => vk::AttachmentLoadOp::CLEAR,
                    LoadOp::DontCare => vk::AttachmentLoadOp::DONT_CARE,
                })
                .store_op(match color_attachment.store_op {
                    StoreOp::Store => vk::AttachmentStoreOp::STORE,
                    StoreOp::DontCare => vk::AttachmentStoreOp::DONT_CARE,
                })
                .clear_value(clear_value);

            let extent = self.swapchain.as_ref()
                .and_then(|w| w.upgrade())
                .map(|s| s.extent)
                .unwrap_or(Extent2D { width: 800, height: 600 });

            let rendering_info = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D { width: extent.width, height: extent.height },
                })
                .layer_count(1)
                .color_attachments(std::slice::from_ref(&rendering_attachment));

            self.device.cmd_begin_rendering(self.command_buffer, &rendering_info);
        }
    }

    fn end_render_pass(&self) {
        unsafe {
            self.device.cmd_end_rendering(self.command_buffer);

            let swapchain = self.swapchain.as_ref().and_then(|w| w.upgrade()).unwrap();
            let image_index = self.current_image_index.get().unwrap() as usize;
            let swapchain_image = swapchain.images[image_index];

            let barrier = vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                .dst_access_mask(vk::AccessFlags2::empty())
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });

            let dependency_info = vk::DependencyInfo::default()
                .image_memory_barriers(std::slice::from_ref(&barrier));

            self.device.cmd_pipeline_barrier2(self.command_buffer, &dependency_info);

            let mut layouts = swapchain.image_layouts.lock().unwrap();
            layouts[image_index] = vk::ImageLayout::PRESENT_SRC_KHR;
        }
    }

    fn set_viewport(&self, width: f32, height: f32) {
        unsafe {
            let viewport = vk::Viewport {
                x: 0.0,
                y: 0.0,
                width,
                height,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            self.device.cmd_set_viewport(self.command_buffer, 0, &[viewport]);
        }
    }

    fn set_scissor(&self, width: u32, height: u32) {
        unsafe {
            let scissor = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width, height },
            };
            self.device.cmd_set_scissor(self.command_buffer, 0, &[scissor]);
        }
    }

    fn bind_pipeline(&self, pipeline: &dyn Pipeline) {
        let vk_pipeline = (pipeline as &dyn Any)
            .downcast_ref::<VulkanPipeline>()
            .expect("pipeline must be VulkanPipeline");

        unsafe {
            self.device.cmd_bind_pipeline(
                self.command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                vk_pipeline.pipeline,
            );
        }
    }

    fn bind_vertex_buffer(&self, buffer: &dyn Buffer) {
        let vk_buffer = (buffer as &dyn Any)
            .downcast_ref::<VulkanBuffer>()
            .expect("buffer must be VulkanBuffer");

        unsafe {
            self.device.cmd_bind_vertex_buffers(
                self.command_buffer,
                0,
                &[vk_buffer.buffer],
                &[0],
            );
        }
    }

    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        unsafe {
            self.device.cmd_draw(
                self.command_buffer,
                vertex_count,
                instance_count,
                first_vertex,
                first_instance,
            );
        }
    }
}

pub struct VulkanQueue {
    device: ash::Device,
    queue: vk::Queue,
}

impl Queue for VulkanQueue {
    fn submit(&self, command_buffers: &[Arc<dyn CommandBuffer>], wait_semaphore: Option<u64>, signal_semaphore: Option<u64>) -> Result<(), RhiError> {
        unsafe {
            let vk_command_buffers: Vec<_> = command_buffers
                .iter()
                .map(|cb| {
                    (&**cb as &dyn Any)
                        .downcast_ref::<VulkanCommandBuffer>()
                        .expect("command buffer must be VulkanCommandBuffer")
                        .command_buffer
                })
                .collect();

            let wait_semaphores = wait_semaphore.map(|s| vec![vk::Semaphore::from_raw(s)]).unwrap_or_default();
            let signal_semaphores = signal_semaphore.map(|s| vec![vk::Semaphore::from_raw(s)]).unwrap_or_default();
            let wait_stages = vec![vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];

            let submit_info = vk::SubmitInfo::default()
                .command_buffers(&vk_command_buffers)
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&wait_stages)
                .signal_semaphores(&signal_semaphores);

            self.device
                .queue_submit(self.queue, &[submit_info], vk::Fence::null())
                .map_err(|e| RhiError::SubmitFailed(format!("Failed to submit command buffer to queue: {}", e)))
        }
    }

    fn wait_idle(&self) -> Result<(), RhiError> {
        unsafe {
            self.device
                .queue_wait_idle(self.queue)
                .map_err(|e| RhiError::InitializationFailed(format!("Queue wait idle failed: {}", e)))
        }
    }
}
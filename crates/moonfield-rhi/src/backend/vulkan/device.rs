use crate::{types::*, *};
use ash::vk::Handle;
use std::any::Any;
use std::sync::Arc;

use super::{VulkanSurface, VulkanSwapchain, VulkanShaderModule, VulkanPipeline, VulkanBuffer, VulkanCommandPool, VulkanQueue};

pub struct VulkanDevice {
    pub instance: ash::Instance,
    pub physical_device: ash::vk::PhysicalDevice,
    pub device: ash::Device,
    pub queue_family_index: u32,
    pub queue: ash::vk::Queue,
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
                Format::B8G8R8A8Unorm => ash::vk::Format::B8G8R8A8_UNORM,
                Format::B8G8R8A8Srgb => ash::vk::Format::B8G8R8A8_SRGB,
                _ => ash::vk::Format::B8G8R8A8_UNORM,
            };

            let create_info = ash::vk::SwapchainCreateInfoKHR::default()
                .surface(vk_surface.surface)
                .min_image_count(desc.image_count)
                .image_format(format)
                .image_color_space(ash::vk::ColorSpaceKHR::SRGB_NONLINEAR)
                .image_extent(ash::vk::Extent2D {
                    width: desc.extent.width,
                    height: desc.extent.height,
                })
                .image_array_layers(1)
                .image_usage(ash::vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(ash::vk::SharingMode::EXCLUSIVE)
                .pre_transform(ash::vk::SurfaceTransformFlagsKHR::IDENTITY)
                .composite_alpha(ash::vk::CompositeAlphaFlagsKHR::OPAQUE)
                .present_mode(ash::vk::PresentModeKHR::FIFO)
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
                    let create_info = ash::vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(ash::vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .components(ash::vk::ComponentMapping::default())
                        .subresource_range(ash::vk::ImageSubresourceRange {
                            aspect_mask: ash::vk::ImageAspectFlags::COLOR,
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

            let semaphore_create_info = ash::vk::SemaphoreCreateInfo::default();
            
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
                image_layouts: std::sync::Mutex::new(vec![ash::vk::ImageLayout::UNDEFINED; image_count]),
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

            let create_info = ash::vk::ShaderModuleCreateInfo::default().code(code);

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

            let entry_name = std::ffi::CString::new("main").unwrap();

            let shader_stages = [
                ash::vk::PipelineShaderStageCreateInfo::default()
                    .stage(ash::vk::ShaderStageFlags::VERTEX)
                    .module(vs.shader_module)
                    .name(&entry_name),
                ash::vk::PipelineShaderStageCreateInfo::default()
                    .stage(ash::vk::ShaderStageFlags::FRAGMENT)
                    .module(fs.shader_module)
                    .name(&entry_name),
            ];

            let binding_descriptions: Vec<_> = desc.vertex_input.bindings.iter().map(|b| {
                ash::vk::VertexInputBindingDescription {
                    binding: b.binding,
                    stride: b.stride,
                    input_rate: match b.input_rate {
                        VertexInputRate::Vertex => ash::vk::VertexInputRate::VERTEX,
                        VertexInputRate::Instance => ash::vk::VertexInputRate::INSTANCE,
                    },
                }
            }).collect();

            let attribute_descriptions: Vec<_> = desc.vertex_input.attributes.iter().map(|a| {
                ash::vk::VertexInputAttributeDescription {
                    location: a.location,
                    binding: a.binding,
                    format: match a.format {
                        VertexFormat::Float32x2 => ash::vk::Format::R32G32_SFLOAT,
                        VertexFormat::Float32x3 => ash::vk::Format::R32G32B32_SFLOAT,
                        VertexFormat::Float32x4 => ash::vk::Format::R32G32B32A32_SFLOAT,
                    },
                    offset: a.offset,
                }
            }).collect();

            let vertex_input_state = ash::vk::PipelineVertexInputStateCreateInfo::default()
                .vertex_binding_descriptions(&binding_descriptions)
                .vertex_attribute_descriptions(&attribute_descriptions);

            let input_assembly_state = ash::vk::PipelineInputAssemblyStateCreateInfo::default()
                .topology(ash::vk::PrimitiveTopology::TRIANGLE_LIST)
                .primitive_restart_enable(false);

            let viewport_state = ash::vk::PipelineViewportStateCreateInfo::default()
                .viewport_count(1)
                .scissor_count(1);

            let rasterization_state = ash::vk::PipelineRasterizationStateCreateInfo::default()
                .depth_clamp_enable(false)
                .rasterizer_discard_enable(false)
                .polygon_mode(ash::vk::PolygonMode::FILL)
                .cull_mode(ash::vk::CullModeFlags::BACK)
                .front_face(ash::vk::FrontFace::CLOCKWISE)
                .depth_bias_enable(false)
                .line_width(1.0);

            let multisample_state = ash::vk::PipelineMultisampleStateCreateInfo::default()
                .rasterization_samples(ash::vk::SampleCountFlags::TYPE_1)
                .sample_shading_enable(false);

            let color_blend_attachment = ash::vk::PipelineColorBlendAttachmentState::default()
                .blend_enable(false)
                .color_write_mask(ash::vk::ColorComponentFlags::RGBA);

            let color_blend_state = ash::vk::PipelineColorBlendStateCreateInfo::default()
                .logic_op_enable(false)
                .attachments(std::slice::from_ref(&color_blend_attachment));

            let dynamic_states = [ash::vk::DynamicState::VIEWPORT, ash::vk::DynamicState::SCISSOR];
            let dynamic_state = ash::vk::PipelineDynamicStateCreateInfo::default()
                .dynamic_states(&dynamic_states);

            let pipeline_layout_create_info = ash::vk::PipelineLayoutCreateInfo::default();
            let pipeline_layout = self
                .device
                .create_pipeline_layout(&pipeline_layout_create_info, None)
                .map_err(|e| RhiError::PipelineCreationFailed(format!("Failed to create pipeline layout: {}", e)))?;

            let format = match desc.render_pass_format {
                Format::B8G8R8A8Unorm => ash::vk::Format::B8G8R8A8_UNORM,
                Format::B8G8R8A8Srgb => ash::vk::Format::B8G8R8A8_SRGB,
                _ => ash::vk::Format::B8G8R8A8_UNORM,
            };

            let color_attachment_format = [format];
            let mut rendering_info = ash::vk::PipelineRenderingCreateInfo::default()
                .color_attachment_formats(&color_attachment_format);

            let pipeline_create_info = ash::vk::GraphicsPipelineCreateInfo::default()
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
                .create_graphics_pipelines(ash::vk::PipelineCache::null(), &[pipeline_create_info], None)
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
                BufferUsage::Vertex => ash::vk::BufferUsageFlags::VERTEX_BUFFER,
                BufferUsage::Index => ash::vk::BufferUsageFlags::INDEX_BUFFER,
                BufferUsage::Uniform => ash::vk::BufferUsageFlags::UNIFORM_BUFFER,
            };

            let buffer_info = ash::vk::BufferCreateInfo::default()
                .size(desc.size)
                .usage(usage)
                .sharing_mode(ash::vk::SharingMode::EXCLUSIVE);

            let buffer = self
                .device
                .create_buffer(&buffer_info, None)
                .map_err(|e| RhiError::BufferCreationFailed(format!("Failed to create buffer: {}", e)))?;

            let mem_requirements = self.device.get_buffer_memory_requirements(buffer);

            let memory_type_index = self.find_memory_type(
                mem_requirements.memory_type_bits,
                ash::vk::MemoryPropertyFlags::HOST_VISIBLE | ash::vk::MemoryPropertyFlags::HOST_COHERENT,
            ).ok_or_else(|| RhiError::BufferCreationFailed("Could not find suitable memory type for buffer".to_string()))?;

            let alloc_info = ash::vk::MemoryAllocateInfo::default()
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
            let pool_info = ash::vk::CommandPoolCreateInfo::default()
                .queue_family_index(self.queue_family_index)
                .flags(ash::vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

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
    pub unsafe fn find_memory_type(&self, type_filter: u32, properties: ash::vk::MemoryPropertyFlags) -> Option<u32> {
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
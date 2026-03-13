use crate::{types::*, *};

mod instance;
mod adapter;
mod device;
mod surface;
mod swapchain;
mod shader_module;
mod pipeline;
mod buffer;
mod command_pool;
mod command_buffer;
mod queue;

pub use instance::*;
pub use adapter::*;
pub use device::*;
pub use surface::*;
pub use swapchain::*;
pub use shader_module::*;
pub use pipeline::*;
pub use buffer::*;
pub use command_pool::*;
pub use command_buffer::*;
pub use queue::*;

pub struct MetalInstance {}

impl MetalInstance {
    pub fn new() -> Result<Self, RhiError> {
        tracing::debug!("Creating Metal instance");
        tracing::info!("Metal instance created successfully");
        Ok(Self {})
    }
}

impl Instance for MetalInstance {
    fn create_surface(&self, window: &winit::window::Window) -> Result<Arc<dyn Surface>, RhiError> {
        tracing::debug!("Creating Metal surface for window");
        tracing::debug!("Metal surface created successfully");
        Ok(Arc::new(MetalSurface {}))
    }

    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>> {
        tracing::debug!("Enumerating Metal adapters");
        unsafe {
            let devices = MTLCopyAllDevices();
            
            let adapters: Vec<Arc<dyn Adapter>> = (0..devices.count())
                .filter_map(|i| {
                    devices.objectAtIndex(i)
                })
                .map(|device| {
                    tracing::debug!("Found Metal device");
                    Arc::new(MetalAdapter {
                        device: Retained::retain(device).unwrap(),
                    }) as Arc<dyn Adapter>
                })
                .collect();
                
            tracing::info!("Found {} Metal adapters", adapters.len());
            adapters
        }
    }
}

pub struct MetalSurface {}

impl Surface for MetalSurface {
    fn get_capabilities(&self, adapter: &dyn Adapter) -> SurfaceCapabilities {
        SurfaceCapabilities {
            formats: vec![Format::B8G8R8A8Unorm, Format::R8G8B8A8Unorm],
            present_modes: vec![PresentMode::Fifo],
            min_image_count: 2,
            max_image_count: 3,
        }
    }
}

pub struct MetalAdapter {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
}

impl std::any::Any for MetalAdapter {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalAdapter>()
    }
}

impl Adapter for MetalAdapter {
    fn request_device(&self) -> Result<Arc<dyn Device>, RhiError> {
        tracing::debug!("Requesting Metal logical device");
        unsafe {
            let queue = self
                .device
                .newCommandQueue()
                .ok_or_else(|| {
                    tracing::error!("Failed to create Metal command queue");
                    RhiError::DeviceCreationFailed("Failed to create Metal command queue".to_string())
                })?;

            tracing::info!("Metal logical device created successfully");
            Ok(Arc::new(MetalDevice {
                device: self.device.clone(),
                queue,
            }))
        }
    }

    fn get_properties(&self) -> AdapterProperties {
        unsafe {
            let name = self.device.name().to_string();
            tracing::debug!("Getting Metal adapter properties: {}", name);
            AdapterProperties {
                name,
                vendor_id: 0,
                device_id: 0,
            }
        }
    }
}

pub struct MetalDevice {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl Device for MetalDevice {
    fn create_swapchain(&self, desc: &SwapchainDescriptor) -> Result<Arc<dyn Swapchain>, RhiError> {
        tracing::debug!("Creating Metal swapchain with format: {:?}, extent: {:?}", desc.format, desc.extent);
        unsafe {
            let layer = CAMetalLayer::new();
            layer.setDevice(Some(&self.device));
            
            let pixel_format = match desc.format {
                Format::B8G8R8A8Unorm => MTLPixelFormat::BGRA8Unorm,
                Format::R8G8B8A8Unorm => MTLPixelFormat::RGBA8Unorm,
                _ => MTLPixelFormat::BGRA8Unorm,
            };
            
            layer.setPixelFormat(pixel_format);
            layer.setDrawableSize(objc2_foundation::CGSize {
                width: desc.extent.width as f64,
                height: desc.extent.height as f64,
            });

            tracing::info!("Metal swapchain created successfully");
            Ok(Arc::new(MetalSwapchain {
                layer,
                format: desc.format,
                extent: desc.extent,
            }))
        }
    }

    fn create_shader_module(&self, desc: &ShaderModuleDescriptor) -> Result<Arc<dyn ShaderModule>, RhiError> {
        tracing::debug!("Creating Metal shader module for stage: {:?}", desc.stage);
        unsafe {
            let source = std::str::from_utf8(desc.code)
                .map_err(|e| {
                    tracing::error!("Invalid UTF-8 in shader source: {}", e);
                    RhiError::ShaderCompilationFailed(ShaderCompilationError::InvalidShaderCode(format!("Invalid UTF-8 in shader source: {}", e)))
                })?;
            
            let ns_source = NSString::from_str(source);
            
            let options = MTLCompileOptions::new();
            
            let library = self
                .device
                .newLibraryWithSource_options_error(&ns_source, &options)
                .map_err(|e| {
                    tracing::error!("Failed to create Metal shader library: {:?}", e);
                    RhiError::ShaderCompilationFailed(ShaderCompilationError::CompilationError(format!("Failed to create Metal shader library: {:?}", e)))
                })?;

            tracing::info!("Metal shader module created successfully for stage: {:?}", desc.stage);
            Ok(Arc::new(MetalShaderModule {
                library,
                stage: desc.stage,
            }))
        }
    }

    fn create_pipeline(&self, desc: &GraphicsPipelineDescriptor) -> Result<Arc<dyn Pipeline>, RhiError> {
        tracing::debug!("Creating Metal graphics pipeline");
        unsafe {
            let vs_any = desc.vertex_shader.as_ref() as &dyn std::any::Any;
            let vs = vs_any.downcast_ref::<MetalShaderModule>().unwrap();

            let fs_any = desc.fragment_shader.as_ref() as &dyn std::any::Any;
            let fs = fs_any.downcast_ref::<MetalShaderModule>().unwrap();

            let pipeline_desc = MTLRenderPipelineDescriptor::new();
            
            let vs_func_name = NSString::from_str("vertex_main");
            let vs_func = vs.library.newFunctionWithName(&vs_func_name)
                .ok_or_else(|| {
                    tracing::error!("Failed to get vertex shader function");
                    RhiError::PipelineCreationFailed("Failed to get vertex shader function".to_string())
                })?;
            pipeline_desc.setVertexFunction(Some(&vs_func));

            let fs_func_name = NSString::from_str("fragment_main");
            let fs_func = fs.library.newFunctionWithName(&fs_func_name)
                .ok_or_else(|| {
                    tracing::error!("Failed to get fragment shader function");
                    RhiError::PipelineCreationFailed("Failed to get fragment shader function".to_string())
                })?;
            pipeline_desc.setFragmentFunction(Some(&fs_func));

            let color_attachment = pipeline_desc
                .colorAttachments()
                .objectAtIndexedSubscript(0);
            
            let pixel_format = match desc.render_pass_format {
                Format::B8G8R8A8Unorm => MTLPixelFormat::BGRA8Unorm,
                Format::R8G8B8A8Unorm => MTLPixelFormat::RGBA8Unorm,
                _ => MTLPixelFormat::BGRA8Unorm,
            };
            
            color_attachment.setPixelFormat(pixel_format);

            let pipeline_state = self
                .device
                .newRenderPipelineStateWithDescriptor_error(&pipeline_desc)
                .map_err(|e| {
                    tracing::error!("Failed to create Metal render pipeline state: {:?}", e);
                    RhiError::PipelineCreationFailed(format!("Failed to create Metal render pipeline state: {:?}", e))
                })?;

            tracing::info!("Metal graphics pipeline created successfully");
            Ok(Arc::new(MetalPipeline {
                pipeline_state,
            }))
        }
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Result<Arc<dyn Buffer>, RhiError> {
        tracing::debug!("Creating Metal buffer with size: {}, memory location: {:?}", desc.size, desc.memory_location);
        unsafe {
            let options = match desc.memory_location {
                MemoryLocation::GpuOnly => MTLResourceOptions::StorageModePrivate,
                MemoryLocation::CpuToGpu => MTLResourceOptions::StorageModeShared,
                MemoryLocation::GpuToCpu => MTLResourceOptions::StorageModeShared,
            };

            let buffer = self
                .device
                .newBufferWithLength_options(desc.size as usize, options)
                .map_err(|_| {
                    tracing::error!("Failed to create Metal buffer with size: {}", desc.size);
                    RhiError::BufferCreationFailed(format!("Failed to create Metal buffer with size: {}", desc.size))
                })?;

            tracing::info!("Metal buffer created successfully with size: {}", desc.size);
            Ok(Arc::new(MetalBuffer { buffer }))
        }
    }

    fn create_command_pool(&self, swapchain: &Arc<dyn Swapchain>) -> Result<Arc<dyn CommandPool>, RhiError> {
        tracing::debug!("Creating Metal command pool");
        tracing::info!("Metal command pool created successfully");
        Ok(Arc::new(MetalCommandPool {
            queue: self.queue.clone(),
        }))
    }

    fn get_queue(&self) -> Arc<dyn Queue> {
        Arc::new(MetalQueue {
            queue: self.queue.clone(),
        })
    }
}

pub struct MetalSwapchain {
    layer: Retained<CAMetalLayer>,
    format: Format,
    extent: Extent2D,
}

impl Swapchain for MetalSwapchain {
    fn acquire_next_image(&self) -> Result<SwapchainImage, RhiError> {
        tracing::debug!("Acquiring next image from Metal swapchain");
        unsafe {
            let drawable = self
                .layer
                .nextDrawable()
                .ok_or_else(|| {
                    tracing::error!("Failed to acquire next drawable from Metal layer");
                    RhiError::AcquireImageFailed("Failed to acquire next drawable from Metal layer".to_string())
                })?;

            tracing::debug!("Successfully acquired next image from Metal swapchain");
            Ok(SwapchainImage {
                index: 0,
                _handle: Retained::as_ptr(&drawable) as usize,
            })
        }
    }

    fn present(&self, image: SwapchainImage) -> Result<(), RhiError> {
        tracing::debug!("Presenting image to Metal swapchain");
        Ok(())
    }

    fn get_format(&self) -> Format {
        self.format
    }

    fn get_extent(&self) -> Extent2D {
        self.extent
    }
}

pub struct MetalShaderModule {
    library: Retained<ProtocolObject<dyn MTLLibrary>>,
    stage: ShaderStage,
}

impl std::any::Any for MetalShaderModule {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalShaderModule>()
    }
}

impl ShaderModule for MetalShaderModule {}

pub struct MetalPipeline {
    pipeline_state: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
}

impl std::any::Any for MetalPipeline {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalPipeline>()
    }
}

impl Pipeline for MetalPipeline {}

pub struct MetalBuffer {
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
}

impl std::any::Any for MetalBuffer {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalBuffer>()
    }
}

impl Buffer for MetalBuffer {
    fn map(&self) -> Result<*mut u8, RhiError> {
        unsafe {
            let ptr = self.buffer.contents();
            Ok(ptr.as_ptr() as *mut u8)
        }
    }

    fn unmap(&self) {}
}

pub struct MetalCommandPool {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl CommandPool for MetalCommandPool {
    fn allocate_command_buffer(&self) -> Result<Arc<dyn CommandBuffer>, RhiError> {
        tracing::debug!("Allocating Metal command buffer");
        unsafe {
            let command_buffer = self
                .queue
                .commandBuffer()
                .ok_or_else(|| {
                    tracing::error!("Failed to allocate Metal command buffer");
                    RhiError::CommandBufferAllocationFailed("Failed to allocate Metal command buffer".to_string())
                })?;

            tracing::info!("Metal command buffer allocated successfully");
            Ok(Arc::new(MetalCommandBuffer {
                command_buffer,
                current_encoder: None,
            }))
        }
    }
}

pub struct MetalCommandBuffer {
    command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    current_encoder: Option<Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>>,
}

impl CommandBuffer for MetalCommandBuffer {
    fn begin(&self) -> Result<(), RhiError> {
        Ok(())
    }

    fn end(&self) -> Result<(), RhiError> {
        Ok(())
    }

    fn begin_render_pass(&self, desc: &RenderPassDescriptor, image: &SwapchainImage) {
        unsafe {
            let render_pass_desc = MTLRenderPassDescriptor::new();
            
            let color_attachment = render_pass_desc
                .colorAttachments()
                .objectAtIndexedSubscript(0);
            
            let clear = &desc.color_attachments[0].clear_value;
            color_attachment.setClearColor(MTLClearColor {
                red: clear[0] as f64,
                green: clear[1] as f64,
                blue: clear[2] as f64,
                alpha: clear[3] as f64,
            });

            let load_action = match desc.color_attachments[0].load_op {
                LoadOp::Load => MTLLoadAction::Load,
                LoadOp::Clear => MTLLoadAction::Clear,
                LoadOp::DontCare => MTLLoadAction::DontCare,
            };
            color_attachment.setLoadAction(load_action);

            let store_action = match desc.color_attachments[0].store_op {
                StoreOp::Store => MTLStoreAction::Store,
                StoreOp::DontCare => MTLStoreAction::DontCare,
            };
            color_attachment.setStoreAction(store_action);
        }
    }

    fn end_render_pass(&self) {
        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.endEncoding();
            }
        }
    }

    fn bind_pipeline(&self, pipeline: &dyn Pipeline) {
        let pipeline_any = pipeline as &dyn std::any::Any;
        let metal_pipeline = pipeline_any.downcast_ref::<MetalPipeline>().unwrap();

        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.setRenderPipelineState(&metal_pipeline.pipeline_state);
            }
        }
    }

    fn bind_vertex_buffer(&self, buffer: &dyn Buffer) {
        let buffer_any = buffer as &dyn std::any::Any;
        let metal_buffer = buffer_any.downcast_ref::<MetalBuffer>().unwrap();

        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.setVertexBuffer_offset_atIndex(Some(&metal_buffer.buffer), 0, 0);
            }
        }
    }

    fn set_viewport(&self, width: f32, height: f32) {}

    fn set_scissor(&self, width: u32, height: u32) {}
    }

    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                    MTLPrimitiveType::Triangle,
                    first_vertex as usize,
                    vertex_count as usize,
                    instance_count as usize,
                    first_instance as usize,
                );
            }
        }
    }


pub struct MetalQueue {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl Queue for MetalQueue {
    fn submit(&self, command_buffers: &[Arc<dyn CommandBuffer>], _wait_semaphore: Option<u64>, _signal_semaphore: Option<u64>) -> Result<(), RhiError> {
        tracing::debug!("Submitting {} command buffers to Metal queue", command_buffers.len());
        for (i, cb) in command_buffers.iter().enumerate() {
            let cb_any = cb.as_ref() as &dyn std::any::Any;
            let metal_cb = cb_any.downcast_ref::<MetalCommandBuffer>().unwrap();
            
            tracing::trace!("Committing command buffer {}", i);
            unsafe {
                metal_cb.command_buffer.commit();
            }
        }
        tracing::debug!("Successfully submitted {} command buffers to Metal queue", command_buffers.len());
        Ok(())
    }

    fn wait_idle(&self) -> Result<(), RhiError> {
        Ok(())
    }
}
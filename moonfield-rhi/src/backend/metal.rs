use crate::{types::*, *};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::*;
use objc2_quartz_core::CAMetalLayer;
use std::sync::Arc;

pub struct MetalInstance {}

impl MetalInstance {
    pub fn new() -> Result<Self, RhiError> {
        Ok(Self {})
    }
}

impl Instance for MetalInstance {
    fn create_surface(&self, window: &winit::window::Window) -> Result<Arc<dyn Surface>, RhiError> {
        Ok(Arc::new(MetalSurface {}))
    }

    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>> {
        unsafe {
            let devices = MTLCopyAllDevices();
            
            (0..devices.count())
                .filter_map(|i| devices.objectAtIndex(i))
                .map(|device| {
                    Arc::new(MetalAdapter {
                        device: Retained::retain(device).unwrap(),
                    }) as Arc<dyn Adapter>
                })
                .collect()
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
        unsafe {
            let queue = self
                .device
                .newCommandQueue()
                .ok_or_else(|| RhiError::DeviceCreationFailed("Failed to create Metal command queue".to_string()))?;

            Ok(Arc::new(MetalDevice {
                device: self.device.clone(),
                queue,
            }))
        }
    }

    fn get_properties(&self) -> AdapterProperties {
        unsafe {
            AdapterProperties {
                name: self.device.name().to_string(),
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

            Ok(Arc::new(MetalSwapchain {
                layer,
                format: desc.format,
                extent: desc.extent,
            }))
        }
    }

    fn create_shader_module(&self, desc: &ShaderModuleDescriptor) -> Result<Arc<dyn ShaderModule>, RhiError> {
        unsafe {
            let source = std::str::from_utf8(desc.code)
                .map_err(|e| RhiError::ShaderCompilationFailed(ShaderCompilationError::InvalidShaderCode(format!("Invalid UTF-8 in shader source: {}", e))))?;
            
            let ns_source = NSString::from_str(source);
            
            let options = MTLCompileOptions::new();
            
            let library = self
                .device
                .newLibraryWithSource_options_error(&ns_source, &options)
                .map_err(|e| RhiError::ShaderCompilationFailed(ShaderCompilationError::CompilationError(format!("Failed to create Metal shader library: {:?}", e))))?;

            Ok(Arc::new(MetalShaderModule {
                library,
                stage: desc.stage,
            }))
        }
    }

    fn create_pipeline(&self, desc: &GraphicsPipelineDescriptor) -> Result<Arc<dyn Pipeline>, RhiError> {
        unsafe {
            let vs_any = desc.vertex_shader.as_ref() as &dyn std::any::Any;
            let vs = vs_any.downcast_ref::<MetalShaderModule>().unwrap();

            let fs_any = desc.fragment_shader.as_ref() as &dyn std::any::Any;
            let fs = fs_any.downcast_ref::<MetalShaderModule>().unwrap();

            let pipeline_desc = MTLRenderPipelineDescriptor::new();
            
            let vs_func_name = NSString::from_str("vertex_main");
            let vs_func = vs.library.newFunctionWithName(&vs_func_name)
                .ok_or_else(|| RhiError::PipelineCreationFailed("Failed to get vertex shader function".to_string()))?;
            pipeline_desc.setVertexFunction(Some(&vs_func));

            let fs_func_name = NSString::from_str("fragment_main");
            let fs_func = fs.library.newFunctionWithName(&fs_func_name)
                .ok_or_else(|| RhiError::PipelineCreationFailed("Failed to get fragment shader function".to_string()))?;
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
                .map_err(|e| RhiError::PipelineCreationFailed(format!("Failed to create Metal render pipeline state: {:?}", e)))?;

            Ok(Arc::new(MetalPipeline {
                pipeline_state,
            }))
        }
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> Result<Arc<dyn Buffer>, RhiError> {
        unsafe {
            let options = match desc.memory_location {
                MemoryLocation::GpuOnly => MTLResourceOptions::StorageModePrivate,
                MemoryLocation::CpuToGpu => MTLResourceOptions::StorageModeShared,
                MemoryLocation::GpuToCpu => MTLResourceOptions::StorageModeShared,
            };

            let buffer = self
                .device
                .newBufferWithLength_options(desc.size as usize, options)
                .ok_or_else(|| RhiError::BufferCreationFailed(format!("Failed to create Metal buffer with size: {}", desc.size)))?;

            Ok(Arc::new(MetalBuffer { buffer }))
        }
    }

    fn create_command_pool(&self, swapchain: &Arc<dyn Swapchain>) -> Result<Arc<dyn CommandPool>, RhiError> {
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
        unsafe {
            let drawable = self
                .layer
                .nextDrawable()
                .ok_or_else(|| RhiError::AcquireImageFailed("Failed to acquire next drawable from Metal layer".to_string()))?;

            Ok(SwapchainImage {
                index: 0,
                _handle: Retained::as_ptr(&drawable) as usize,
            })
        }
    }

    fn present(&self, image: SwapchainImage) -> Result<(), RhiError> {
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
        unsafe {
            let command_buffer = self
                .queue
                .commandBuffer()
                .ok_or_else(|| RhiError::CommandBufferAllocationFailed("Failed to allocate Metal command buffer".to_string()))?;

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
}

pub struct MetalQueue {
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl Queue for MetalQueue {
    fn submit(&self, command_buffers: &[Arc<dyn CommandBuffer>], _wait_semaphore: Option<u64>, _signal_semaphore: Option<u64>) -> Result<(), RhiError> {
        for cb in command_buffers {
            let cb_any = cb.as_ref() as &dyn std::any::Any;
            let metal_cb = cb_any.downcast_ref::<MetalCommandBuffer>().unwrap();
            
            unsafe {
                metal_cb.command_buffer.commit();
            }
        }
        Ok(())
    }

    fn wait_idle(&self) -> Result<(), RhiError> {
        Ok(())
    }
}
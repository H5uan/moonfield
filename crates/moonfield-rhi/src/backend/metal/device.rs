use crate::{types::*, Device, Swapchain, ShaderModule, Pipeline, Buffer, CommandPool, Queue, RhiError};
use std::sync::Arc;

// Import tracing for logging
use tracing;

pub struct MetalDevice {
    pub device: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLDevice>>,
    pub queue: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLCommandQueue>>,
}

impl Device for MetalDevice {
    fn create_swapchain(&self, desc: &SwapchainDescriptor) -> Result<Arc<dyn Swapchain>, RhiError> {
        tracing::debug!("Creating Metal swapchain with format: {:?}, extent: {:?}", desc.format, desc.extent);
        unsafe {
            let layer = objc2_quartz_core::CAMetalLayer::new();
            layer.setDevice(Some(&self.device));
            
            let pixel_format = match desc.format {
                Format::B8G8R8A8Unorm => objc2_metal::MTLPixelFormat::BGRA8Unorm,
                Format::R8G8B8A8Unorm => objc2_metal::MTLPixelFormat::RGBA8Unorm,
                _ => objc2_metal::MTLPixelFormat::BGRA8Unorm,
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
            
            let ns_source = objc2_foundation::NSString::from_str(source);
            
            let options = objc2_metal::MTLCompileOptions::new();
            
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
            let vs = vs_any.downcast_ref::<super::shader_module::MetalShaderModule>().unwrap();

            let fs_any = desc.fragment_shader.as_ref() as &dyn std::any::Any;
            let fs = fs_any.downcast_ref::<super::shader_module::MetalShaderModule>().unwrap();

            let pipeline_desc = objc2_metal::MTLRenderPipelineDescriptor::new();
            
            let vs_func_name = objc2_foundation::NSString::from_str("vertex_main");
            let vs_func = vs.library.newFunctionWithName(&vs_func_name)
                .ok_or_else(|| {
                    tracing::error!("Failed to get vertex shader function");
                    RhiError::PipelineCreationFailed("Failed to get vertex shader function".to_string())
                })?;
            pipeline_desc.setVertexFunction(Some(&vs_func));

            let fs_func_name = objc2_foundation::NSString::from_str("fragment_main");
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
                Format::B8G8R8A8Unorm => objc2_metal::MTLPixelFormat::BGRA8Unorm,
                Format::R8G8B8A8Unorm => objc2_metal::MTLPixelFormat::RGBA8Unorm,
                _ => objc2_metal::MTLPixelFormat::BGRA8Unorm,
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
                MemoryLocation::GpuOnly => objc2_metal::MTLResourceOptions::StorageModePrivate,
                MemoryLocation::CpuToGpu => objc2_metal::MTLResourceOptions::StorageModeShared,
                MemoryLocation::GpuToCpu => objc2_metal::MTLResourceOptions::StorageModeShared,
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
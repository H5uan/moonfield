pub mod backend;
pub mod types;

use raw_window_handle::HasDisplayHandle;
use std::any::Any;
use std::sync::Arc;
use types::*;

pub trait Instance {
    fn create_surface(&self, window: &winit::window::Window) -> Result<Arc<dyn Surface>, RhiError>;
    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>>;
}

pub trait Adapter: Any {
    fn request_device(&self) -> Result<Arc<dyn Device>, RhiError>;
    fn get_properties(&self) -> AdapterProperties;
}

pub trait Device {
    fn create_swapchain(&self, desc: &SwapchainDescriptor) -> Result<Arc<dyn Swapchain>, RhiError>;
    fn create_shader_module(&self, desc: &ShaderModuleDescriptor) -> Result<Arc<dyn ShaderModule>, RhiError>;
    fn create_pipeline(&self, desc: &GraphicsPipelineDescriptor) -> Result<Arc<dyn Pipeline>, RhiError>;
    fn create_buffer(&self, desc: &BufferDescriptor) -> Result<Arc<dyn Buffer>, RhiError>;
    fn create_command_pool(&self, swapchain: &Arc<dyn Swapchain>) -> Result<Arc<dyn CommandPool>, RhiError>;
    fn get_queue(&self) -> Arc<dyn Queue>;
}

pub trait Surface: Any {
    fn get_capabilities(&self, adapter: &dyn Adapter) -> SurfaceCapabilities;
}

pub trait Swapchain: Any {
    fn acquire_next_image(&self) -> Result<SwapchainImage, RhiError>;
    fn present(&self, image: SwapchainImage) -> Result<(), RhiError>;
    fn get_format(&self) -> Format;
    fn get_extent(&self) -> Extent2D;
}

pub trait ShaderModule: Any {}

pub trait Pipeline: Any {}

pub trait Buffer: Any {
    fn map(&self) -> Result<*mut u8, RhiError>;
    fn unmap(&self);
}

pub trait CommandPool {
    fn allocate_command_buffer(&self) -> Result<Arc<dyn CommandBuffer>, RhiError>;
}

pub trait CommandBuffer: Any {
    fn begin(&self) -> Result<(), RhiError>;
    fn end(&self) -> Result<(), RhiError>;
    fn begin_render_pass(&self, desc: &RenderPassDescriptor, image: &SwapchainImage);
    fn end_render_pass(&self);
    fn set_viewport(&self, width: f32, height: f32);
    fn set_scissor(&self, width: u32, height: u32);
    fn bind_pipeline(&self, pipeline: &dyn Pipeline);
    fn bind_vertex_buffer(&self, buffer: &dyn Buffer);
    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32);
}

pub trait Queue {
    fn submit(&self, command_buffers: &[Arc<dyn CommandBuffer>], wait_semaphore: Option<u64>, signal_semaphore: Option<u64>) -> Result<(), RhiError>;
    fn wait_idle(&self) -> Result<(), RhiError>;
}

pub fn create_instance(backend: Backend) -> Result<Arc<dyn Instance>, RhiError> {
    match backend {
        #[cfg(feature = "vulkan")]
        Backend::Vulkan => {
            #[cfg(windows)]
            {
                Ok(Arc::new(backend::vulkan::VulkanInstance::new()?))
            }
            #[cfg(not(windows))]
            {
                Err(RhiError::BackendNotSupported)
            }
        }
        #[cfg(target_os = "macos")]
        Backend::Metal => Ok(Arc::new(backend::metal::MetalInstance::new()?)),
        #[cfg(all(windows, feature = "dx12"))]
        Backend::Dx12 => Ok(Arc::new(backend::dx12::Dx12Instance::new()?)),
        _ => Err(RhiError::BackendNotSupported),
    }
}

pub fn create_instance_with_window(backend: Backend, window: &winit::window::Window) -> Result<Arc<dyn Instance>, RhiError> {
    match backend {
        #[cfg(feature = "vulkan")]
        Backend::Vulkan => {
            let display = window.display_handle()
                .map_err(|e| RhiError::InitializationFailed(e.to_string()))?
                .as_raw();
            Ok(Arc::new(backend::vulkan::VulkanInstance::new_with_display(display)?))
        }
        #[cfg(target_os = "macos")]
        Backend::Metal => Ok(Arc::new(backend::metal::MetalInstance::new()?)),
        #[cfg(all(windows, feature = "dx12"))]
        Backend::Dx12 => Ok(Arc::new(backend::dx12::Dx12Instance::new()?)),
        _ => Err(RhiError::BackendNotSupported),
    }
}
use std::sync::Arc;
use crate::{types::*, Device, Swapchain, ShaderModule, Pipeline, Buffer, CommandPool, Queue, RhiError};

// Import Windows-specific DirectX 12 types
use windows::{
    core::*,
    Win32::Graphics::Direct3D12::*,
    Win32::Graphics::Dxgi::*, 
    Win32::Foundation::*,
};

// Import tracing for logging
use tracing;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

pub struct Dx12Device {
    pub device: ID3D12Device,
    pub queue: ID3D12CommandQueue,
}

impl Dx12Device {
    pub fn new(d3d12_device: &ID3D12Device) -> StdResult<Self, RhiError> {
        // Create command queue
        let queue = unsafe {
            let queue_desc = D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: 0, // D3D12_COMMAND_QUEUE_PRIORITY_NORMAL
                Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
                NodeMask: 0,
            };
            
            let queue: ID3D12CommandQueue = d3d12_device.CreateCommandQueue(&queue_desc)
                .map_err(|e| RhiError::DeviceCreationFailed(format!("Failed to create command queue: {}", e)))?;
            
            queue
        };
        
        Ok(Dx12Device { 
            device: d3d12_device.clone(),
            queue,
        })
    }
    
    pub fn get_device(&self) -> &ID3D12Device {
        &self.device
    }
    
    pub fn get_queue(&self) -> &ID3D12CommandQueue {
        &self.queue
    }
}

impl Device for Dx12Device {
    fn create_swapchain(&self, desc: &SwapchainDescriptor) -> StdResult<Arc<dyn Swapchain>, RhiError> {
        // Extract the surface as Dx12Surface to get the HWND
        let surface_any = desc.surface.as_any();
        let dx12_surface = surface_any.downcast_ref::<super::surface::Dx12Surface>()
            .ok_or_else(|| RhiError::SwapchainCreationFailed("Invalid surface type for DX12".to_string()))?;
            
        let dx12_swapchain = super::swapchain::Dx12Swapchain::new(self, dx12_surface, desc)?;
        Ok(Arc::new(dx12_swapchain) as Arc<dyn Swapchain>)
    }

    fn create_shader_module(&self, desc: &ShaderModuleDescriptor) -> StdResult<Arc<dyn ShaderModule>, RhiError> {
        let dx12_shader = super::shader_module::Dx12ShaderModule::new(desc)?;
        Ok(Arc::new(dx12_shader) as Arc<dyn ShaderModule>)
    }

    fn create_pipeline(&self, desc: &GraphicsPipelineDescriptor) -> StdResult<Arc<dyn Pipeline>, RhiError> {
        let dx12_pipeline = super::pipeline::Dx12Pipeline::new(self, desc)?;
        Ok(Arc::new(dx12_pipeline) as Arc<dyn Pipeline>)
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> StdResult<Arc<dyn Buffer>, RhiError> {
        let dx12_buffer = super::buffer::Dx12Buffer::new(self, desc)?;
        Ok(Arc::new(dx12_buffer) as Arc<dyn Buffer>)
    }

    fn create_command_pool(&self, _swapchain: &Arc<dyn Swapchain>) -> StdResult<Arc<dyn CommandPool>, RhiError> {
        let dx12_command_pool = super::command_pool::Dx12CommandPool::new(self)?;
        Ok(Arc::new(dx12_command_pool) as Arc<dyn CommandPool>)
    }

    fn get_queue(&self) -> Arc<dyn Queue> {
        Arc::new(super::queue::Dx12Queue::new(&self.queue))
    }
}
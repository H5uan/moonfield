use std::sync::Arc;
use crate::{types::*, CommandPool, CommandBuffer, RhiError};

// Import Windows-specific DirectX 12 types
use windows::{
    core::*,
    Win32::Graphics::Direct3D12::*, 
    Win32::Foundation::*,
};

// Import tracing for logging
use tracing;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

pub struct Dx12CommandPool {
    pub device: ID3D12Device,
    pub queue: ID3D12CommandQueue,
    pub command_allocator: ID3D12CommandAllocator,
}

impl Dx12CommandPool {
    pub fn new(device: &super::device::Dx12Device) -> StdResult<Self, RhiError> {
        unsafe {
            let command_allocator = device.device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .map_err(|e| RhiError::CommandPoolCreationFailed(format!("Failed to create command allocator: {}", e)))?;
            
            Ok(Dx12CommandPool {
                device: device.device.clone(),
                queue: device.queue.clone(),
                command_allocator,
            })
        }
    }
}

impl CommandPool for Dx12CommandPool {
    fn allocate_command_buffer(&self) -> StdResult<Arc<dyn CommandBuffer>, RhiError> {
        let command_buffer = super::command_buffer::Dx12CommandBuffer::new(&self.device, &self.command_allocator)?;
        Ok(Arc::new(command_buffer) as Arc<dyn CommandBuffer>)
    }
}
use std::sync::Arc;
use crate::{types::*, CommandBuffer, RenderPassDescriptor, SwapchainImage, RhiError};

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

use std::sync::RwLock;

pub struct Dx12CommandBuffer {
    pub command_list: RwLock<Option<ID3D12GraphicsCommandList>>,
    pub command_allocator: ID3D12CommandAllocator,
    pub device: ID3D12Device,
    pub is_recording: RwLock<bool>,
}

impl Dx12CommandBuffer {
    pub fn new(device: &ID3D12Device, command_allocator: &ID3D12CommandAllocator) -> StdResult<Self, RhiError> {
        unsafe {
            let command_list = device.CreateCommandList(
                0,                                    // node mask
                D3D12_COMMAND_LIST_TYPE_DIRECT,       // command list type
                command_allocator,                     // initial allocator
                None,                                  // initial pipeline state (none for now)
            ).map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to create command list: {}", e)))?;
            
            // Close the command list since new command lists are created in recording state
            command_list.Close().ok(); // It's okay if closing fails here
            
            Ok(Dx12CommandBuffer {
                command_list: RwLock::new(Some(command_list)),
                command_allocator: command_allocator.clone(),
                device: device.clone(),
                is_recording: RwLock::new(false),
            })
        }
    }
    
    pub fn get_command_list(&self) -> Result<std::sync::RwLockReadGuard<Option<ID3D12GraphicsCommandList>>, RhiError> {
        self.command_list.read().map_err(|_| RhiError::CommandBufferAllocationFailed("Failed to lock command list".to_string()))
    }
    
    pub fn get_command_list_mut(&self) -> Result<std::sync::RwLockWriteGuard<Option<ID3D12GraphicsCommandList>>, RhiError> {
        self.command_list.write().map_err(|_| RhiError::CommandBufferAllocationFailed("Failed to lock command list".to_string()))
    }
}

impl CommandBuffer for Dx12CommandBuffer {
    fn begin(&self) -> StdResult<(), RhiError> {
        let mut is_recording = self.is_recording.write().unwrap();
        if *is_recording {
            return Err(RhiError::CommandBufferAllocationFailed("Command buffer already recording".to_string()));
        }
        
        unsafe {
            let mut cmd_list_guard = self.command_list.write().unwrap();
            
            // Reset the command allocator
            cmd_list_guard.as_ref().unwrap().Reset(&self.command_allocator, None)
                .map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to reset command list: {}", e)))?;
        }
        
        *is_recording = true;
        Ok(())
    }

    fn end(&self) -> StdResult<(), RhiError> {
        let mut is_recording = self.is_recording.write().unwrap();
        if !*is_recording {
            return Err(RhiError::CommandBufferAllocationFailed("Command buffer not recording".to_string()));
        }
        
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            cmd_list_guard.as_ref().unwrap().Close()
                .map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to close command list: {}", e)))?;
        }
        
        *is_recording = false;
        Ok(())
    }

    fn begin_render_pass(&self, _desc: &RenderPassDescriptor, _image: &SwapchainImage) {
        // In DX12, render passes are handled differently than in Vulkan/Metal
        // We set up render targets here
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // TODO: Actually set up render targets here
                // This would involve creating and binding RTVs based on the image
            }
        }
    }
    
    fn end_render_pass(&self) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // In DX12, we typically don't need an explicit end_render_pass
                // but we can add any necessary barriers here
            }
        }
    }
    
    fn set_viewport(&self, width: f32, height: f32) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                let viewport = D3D12_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: width,
                    Height: height,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                };
                
                command_list.RSSetViewports(&[viewport]);
            }
        }
    }
    
    fn set_scissor(&self, width: u32, height: u32) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                let rect = D3D12_RECT {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                };
                
                command_list.RSSetScissorRects(&[rect]);
            }
        }
    }
    
    fn bind_pipeline(&self, pipeline: &dyn crate::Pipeline) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // Downcast the pipeline to Dx12Pipeline to get the actual D3D12 object
                let pipeline_any = pipeline.as_any();
                if let Some(dx12_pipeline) = pipeline_any.downcast_ref::<super::pipeline::Dx12Pipeline>() {
                    command_list.SetPipelineState(&dx12_pipeline.pipeline_state);
                }
            }
        }
    }
    
    fn bind_vertex_buffer(&self, buffer: &dyn crate::Buffer) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // Downcast the buffer to Dx12Buffer to get the actual D3D12 resource
                let buffer_any = buffer.as_any();
                if let Some(dx12_buffer) = buffer_any.downcast_ref::<super::buffer::Dx12Buffer>() {
                    let gpu_virtual_address = dx12_buffer.buffer.GetGPUVirtualAddress();
                    
                    let vertex_buffer_view = D3D12_VERTEX_BUFFER_VIEW {
                        BufferLocation: gpu_virtual_address,
                        SizeInBytes: dx12_buffer.size as u32,
                        StrideInBytes: 0, // This would normally come from the vertex layout
                    };
                    
                    command_list.IASetVertexBuffers(0, &[vertex_buffer_view]);
                }
            }
        }
    }
    
    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                command_list.DrawInstanced(vertex_count, instance_count, first_vertex, first_instance);
            }
        }
    }
}
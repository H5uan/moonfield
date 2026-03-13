use crate::{types::*, Buffer, BufferUsage, MemoryLocation, RhiError};

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
use std::sync::Mutex;

pub struct Dx12Buffer {
    pub buffer: ID3D12Resource,
    pub size: u64,
    pub usage: BufferUsage,
    pub memory_location: MemoryLocation,
    pub mapped_ptr: Mutex<Option<*mut u8>>,
}

impl Dx12Buffer {
    pub fn new(device: &super::device::Dx12Device, desc: &BufferDescriptor) -> StdResult<Self, RhiError> {
        unsafe {
            let heap_properties = match desc.memory_location {
                MemoryLocation::GpuOnly => D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_DEFAULT,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                },
                MemoryLocation::CpuToGpu | MemoryLocation::GpuToCpu => D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD, // For CPU to GPU transfer
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                },
            };

            let resource_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 0,
                Width: desc.size,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_NONE,
            };

            let mut buffer: Option<ID3D12Resource> = None;
            let hr = device.device.CreateCommittedResource(
                &heap_properties,
                D3D12_HEAP_FLAG_NONE,
                &resource_desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut buffer,
            );

            if hr.is_err() {
                return Err(RhiError::BufferCreationFailed(format!("Failed to create buffer: {}", hr.err().unwrap())));
            }

            let buffer = buffer.ok_or_else(|| RhiError::BufferCreationFailed("Buffer creation returned null".to_string()))?;

            Ok(Dx12Buffer {
                buffer,
                size: desc.size,
                usage: desc.usage.clone(),
                memory_location: desc.memory_location.clone(),
                mapped_ptr: Mutex::new(None),
            })
        }
    }
    
    pub fn get_resource(&self) -> &ID3D12Resource {
        &self.buffer
    }
}

impl Buffer for Dx12Buffer {
    fn map(&self) -> StdResult<*mut u8, RhiError> {
        if self.memory_location == MemoryLocation::GpuOnly {
            return Err(RhiError::MapFailed("Cannot map GPU-only buffer".to_string()));
        }
        
        unsafe {
            let mut mapped_ptr = self.mapped_ptr.lock().unwrap();
            if mapped_ptr.is_none() {
                let range = D3D12_RANGE {
                    Begin: 0,
                    End: self.size as usize,
                };
                
                let ptr = self.buffer.Map(0, Some(&range))
                    .map_err(|e| RhiError::MapFailed(format!("Failed to map buffer: {}", e)))? as *mut u8;
                
                *mapped_ptr = Some(ptr);
            }
            
            Ok(mapped_ptr.unwrap())
        }
    }

    fn unmap(&self) {
        unsafe {
            let mut mapped_ptr = self.mapped_ptr.lock().unwrap();
            if let Some(ptr) = *mapped_ptr {
                let range = D3D12_RANGE {
                    Begin: 0,
                    End: self.size as usize,
                };
                
                self.buffer.Unmap(0, Some(&range));
                *mapped_ptr = None;
            }
        }
    }
}
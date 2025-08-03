use std::{ffi::c_void, rc::Weak};

use bytemuck::{Pod, Zeroable};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLBuffer, MTLDevice, MTLResourceOptions, MTLStorageMode};

use crate::{
    buffer::{
        AccessPattern, BufferDescriptor, BufferKind, BufferPlacement, GPUBuffer,
    },
    error::{GraphicsError, MetalError},
    metal_backend::MetalGraphicsBackend,
};

pub struct MetalBuffer {
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    backend: Weak<MetalGraphicsBackend>,
    desc: BufferDescriptor,
}

impl MetalBuffer {
    /// Write typed data to the buffer using bytemuck for safe byte conversion
    pub fn write_typed_data<T: Pod>(
        &self, data: &[T], offset: usize,
    ) -> Result<(), GraphicsError> {
        let bytes = bytemuck::cast_slice(data);
        self.write_data(bytes, offset)
    }

    /// Read typed data from the buffer using bytemuck for safe byte conversion
    pub fn read_typed_data<T: Pod + Zeroable>(
        &self, data: &mut [T], offset: usize,
    ) -> Result<(), GraphicsError> {
        let bytes = bytemuck::cast_slice_mut(data);
        self.read_data(bytes, offset)
    }

    /// Write a single typed value to the buffer
    pub fn write_value<T: Pod>(
        &self, value: &T, offset: usize,
    ) -> Result<(), GraphicsError> {
        let bytes = bytemuck::bytes_of(value);
        self.write_data(bytes, offset)
    }

    /// Read a single typed value from the buffer
    pub fn read_value<T: Pod + Zeroable>(
        &self, offset: usize,
    ) -> Result<T, GraphicsError> {
        let mut value = T::zeroed();
        let bytes = bytemuck::bytes_of_mut(&mut value);
        self.read_data(bytes, offset)?;
        Ok(value)
    }

    fn descriptor_to_metal_options(
        desc: &BufferDescriptor,
    ) -> MTLResourceOptions {
        let base_options = match desc.placement {
            // Apple silicon have unified memory arch, so StorageModeManaged will fallback to StorageModeShared
            BufferPlacement::GpuOnly => MTLResourceOptions::StorageModePrivate,
            BufferPlacement::Shared => MTLResourceOptions::StorageModeShared,
        };

        let mut options = base_options;

        match desc.access_pattern {
            AccessPattern::WriteEveryFrameReadMany => {
                options |= MTLResourceOptions::CPUCacheModeWriteCombined;
            }
            _ => {}
        }

        options
    }

    pub fn new(
        backend: &MetalGraphicsBackend, desc: BufferDescriptor,
    ) -> Result<Self, GraphicsError> {
        // Convert BufferPlacement and AccessPattern to MTLResourceOptions
        let resource_options = Self::descriptor_to_metal_options(&desc);

        // Create the Metal buffer
        let device = backend.device();
        let buffer = device
            .newBufferWithLength_options(desc.size, resource_options)
            .ok_or_else(|| {
                GraphicsError::MetalError(MetalError::BufferCreationError(
                    format!(
                        "Failed to create Metal buffer of size {} bytes with placement {:?}",
                        desc.size, desc.placement
                    ),
                ))
            })?;

        // Create weak reference to backend
        let backend_weak = Weak::new(); // This should be properly set by the caller

        Ok(Self {
            buffer,
            backend: backend_weak,
            desc,
        })
    }
}

impl GPUBuffer for MetalBuffer {
    fn allocated_size(&self) -> usize {
        // Metal buffer returns the actual allocated size
        self.buffer.length()
    }

    fn write_data(
        &self, data: &[u8], offset: usize,
    ) -> Result<(), GraphicsError> {
        // Check bounds
        if offset + data.len() > self.allocated_size() {
            return Err(GraphicsError::MetalError(MetalError::BufferCreationError(
                format!(
                    "Write data exceeds buffer bounds: offset {} + size {} > buffer size {}",
                    offset,
                    data.len(),
                    self.allocated_size()
                ),
            )));
        }

        // For shared memory buffers, we can directly write to the buffer
        if self.desc.placement == BufferPlacement::Shared {
            unsafe {
                let buffer_ptr = self.buffer.contents().as_ptr().cast::<u8>();
                
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    buffer_ptr.add(offset),
                    data.len(),
                );
            }
        } else {
            // For GPU-only buffers, we would need a staging buffer or command buffer
            // For now, return an error as this requires more complex implementation
            return Err(GraphicsError::MetalError(MetalError::BufferCreationError(
                "Writing to GPU-only buffers not yet implemented".to_string(),
            )));
        }

        Ok(())
    }

    fn read_data(
        &self, data: &mut [u8], offset: usize,
    ) -> Result<(), GraphicsError> {
        // Check bounds
        if offset + data.len() > self.allocated_size() {
            return Err(GraphicsError::MetalError(MetalError::BufferCreationError(
                format!(
                    "Read data exceeds buffer bounds: offset {} + size {} > buffer size {}",
                    offset,
                    data.len(),
                    self.allocated_size()
                ),
            )));
        }

        // Only shared memory buffers can be read directly
        if self.desc.placement == BufferPlacement::Shared {
            unsafe {
                let buffer_ptr = self.buffer.contents().as_ptr().cast::<u8>();
                
                std::ptr::copy_nonoverlapping(
                    buffer_ptr.add(offset),
                    data.as_mut_ptr(),
                    data.len(),
                );
            }
        } else {
            return Err(GraphicsError::MetalError(MetalError::BufferCreationError(
                "Cannot read from GPU-only buffers".to_string(),
            )));
        }

        Ok(())
    }
}

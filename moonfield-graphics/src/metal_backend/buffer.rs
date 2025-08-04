use std::{
    cell::Cell,
    ffi::c_void,
    fmt::format,
    rc::{Rc, Weak},
};

use bytemuck::{Pod, Zeroable};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLBuffer, MTLDevice, MTLResourceOptions, MTLStorageMode};

use crate::{
    backend,
    buffer::{
        self, BufferAccessPattern, BufferKind, GPUBuffer, GPUBufferDescriptor,
    },
    error::{GraphicsError, MetalError},
    metal_backend::MetalGraphicsBackend,
};

pub struct MetalBuffer {
    pub backend: Weak<MetalGraphicsBackend>,
    pub buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    pub size: Cell<usize>,
    pub kind: BufferKind,
    pub access_pattern: BufferAccessPattern,
}

impl MetalBuffer {
    fn access_pattern_to_resource_options(
        access_pattern: BufferAccessPattern,
    ) -> MTLResourceOptions {
        match access_pattern {
            BufferAccessPattern::Stream => {
                MTLResourceOptions::StorageModeShared
                    | MTLResourceOptions::CPUCacheModeWriteCombined
            }
            BufferAccessPattern::Dynamic => {
                MTLResourceOptions::StorageModeShared
                    | MTLResourceOptions::CPUCacheModeWriteCombined
            }
            BufferAccessPattern::GpuReadOnly => {
                MTLResourceOptions::StorageModePrivate
            }
            BufferAccessPattern::GpuWriteCpuRead => {
                MTLResourceOptions::StorageModeShared
                    | MTLResourceOptions::CPUCacheModeDefaultCache
            }
            BufferAccessPattern::GpuInternal => {
                MTLResourceOptions::StorageModePrivate
            }
        }
    }

    pub fn new(
        backend: &MetalGraphicsBackend, desc: GPUBufferDescriptor,
    ) -> Result<Self, GraphicsError> {
        let GPUBufferDescriptor {
            #[allow(unused_variables)]
            name,
            size,
            kind,
            access_pattern,
        } = desc;

        let resource_options =
            Self::access_pattern_to_resource_options(access_pattern);
        let buffer = backend
            .device()
            .newBufferWithLength_options(size, resource_options)
            .ok_or_else(|| {
                GraphicsError::MetalError(MetalError::BufferCreationError(
                    format!("Failed to create buffer with size {}", size),
                ))
            })?;

        Ok(Self {
            backend: backend.weak(),
            buffer,
            size: Cell::new(size),
            kind,
            access_pattern,
        })
    }
}

impl GPUBuffer for MetalBuffer {
    fn allocated_size(&self) -> usize {
        self.size.get()
    }

    fn access_pattern(&self) -> BufferAccessPattern {
        self.access_pattern
    }

    fn kind(&self) -> BufferKind {
        self.kind
    }

    fn read_data(&self, data: &mut [u8]) -> Result<(), GraphicsError> {
        let Some(_backend) = self.backend.upgrade() else {
            return Err(GraphicsError::BackendUnavailable);
        };
        match self.access_pattern {
            BufferAccessPattern::GpuReadOnly
            | BufferAccessPattern::GpuInternal => {
                return Err(GraphicsError::InvalidOperation(
                    "Buffer not readable by CPU".to_string(),
                ));
            }
            _ => {}
        }

        let read_size = data.len().min(self.size.get());
        if read_size == 0 {
            return Ok(());
        }

        unsafe {
            let buffer_ptr = self.buffer.contents().as_ptr() as *const u8;

            std::ptr::copy_nonoverlapping(
                buffer_ptr,
                data.as_mut_ptr(),
                read_size,
            );
        }

        Ok(())
    }

    fn write_data(&self, data: &[u8]) -> Result<(), GraphicsError> {
        let Some(_backend) = self.backend.upgrade() else {
            return Err(GraphicsError::BackendUnavailable);
        };
        match self.access_pattern {
            BufferAccessPattern::GpuReadOnly
            | BufferAccessPattern::GpuInternal => {
                return Err(GraphicsError::InvalidOperation(
                    "Buffer not readable by CPU".to_string(),
                ));
            }
            _ => {}
        }

        if data.len() > self.size.get() {
            return Err(GraphicsError::BufferOverflow);
        }

        if data.is_empty() {
            return Ok(());
        }

        unsafe {
            let buffer_ptr = self.buffer.contents().as_ptr() as *mut u8;

            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                buffer_ptr,
                data.len(),
            );
        }

        Ok(())
    }
}

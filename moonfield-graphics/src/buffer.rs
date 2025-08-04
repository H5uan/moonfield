use bytemuck::Pod;
use moonfield_core::{any_ext_for, array_as_u8_slice, array_as_u8_slice_mut};

use crate::error::GraphicsError;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BufferAccessPattern {
    Stream,
    Dynamic,
    GpuReadOnly, // static
    GpuWriteCpuRead,
    GpuInternal,
}

/// Buffer type definition for modern graphics APIs.
/// Covers all use cases in Vulkan and Metal.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BufferKind {
    Vertex,
    Index,
    Uniform,
    Storage,
    Indirect,
}

/// Buffer creation descriptor for Vulkan and Metal backends
#[derive(Copy, Clone, Debug)]
pub struct GPUBufferDescriptor<'a> {
    pub name: &'a str,
    /// Size of the buffer in bytes
    pub size: usize,
    /// Buffer kind like vertex, index, uniform
    pub kind: BufferKind,
    /// Buffer access pattern
    pub access_pattern: BufferAccessPattern,
}

any_ext_for!(GPUBuffer => GPUBufferAsAny);

/// Modern GPU buffer trait optimized for Vulkan and Metal
pub trait GPUBuffer {
    /// Returns the actual allocated size (may be larger than requested due to alignment)
    fn allocated_size(&self) -> usize;

    fn kind(&self) -> BufferKind;

    fn access_pattern(&self) -> BufferAccessPattern;

    /// Writes data to the buffer
    fn write_data(&self, data: &[u8]) -> Result<(), GraphicsError>;

    /// Reads data from buffer (only for CPU-accessible buffers)
    fn read_data(&self, data: &mut [u8]) -> Result<(), GraphicsError>;
}

impl dyn GPUBuffer {
    pub fn write_typed_data<T: Pod>(
        &self, data: &[T],
    ) -> Result<(), GraphicsError> {
        let untyped_data = array_as_u8_slice(data);
        GPUBuffer::write_data(self, untyped_data)
    }

    pub fn read_typed_data<T: Pod>(
        &self, data: &mut [T],
    ) -> Result<(), GraphicsError> {
        let untyped_data = array_as_u8_slice_mut(data);
        GPUBuffer::read_data(self, untyped_data)
    }
}

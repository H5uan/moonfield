use crate::error::GraphicsError;

/// Buffer usage pattern for modern GPU memory management.
/// Designed specifically for Vulkan and Metal memory models.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BufferPlacement {
    GpuOnly,
    Shared,
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

/// Memory access pattern hints for optimization
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum AccessPattern {
    /// Data written once, read many times (static geometry)
    WriteOnceReadMany,
    /// Data updated every frame (dynamic uniforms)
    WriteEveryFrameReadMany,
    /// Data updated occasionally, read many times (animated models)
    WriteOccasionallyReadMany,
    /// Data written by GPU, read by CPU (query results, screenshots)
    WriteGpuReadCpu,
    /// Data written by CPU, processed by GPU compute (simulation input)
    WriteCpuProcessGpu,
}

/// Buffer creation descriptor for Vulkan and Metal backends
#[derive(Copy, Clone, Debug)]
pub struct BufferDescriptor {
    /// Size of the buffer in bytes
    pub size: usize,
    /// Buffer kind
    pub buffer_kind: BufferKind,
    /// Memory usage pattern
    pub placement: BufferPlacement,
    /// Access pattern hint for optimization
    pub access_pattern: AccessPattern,
}

impl Default for BufferDescriptor {
    fn default() -> Self {
        Self {
            size: 0,
            buffer_kind: BufferKind::Storage,
            placement: BufferPlacement::GpuOnly,
            access_pattern: AccessPattern::WriteEveryFrameReadMany,
        }
    }
}

/// Modern GPU buffer trait optimized for Vulkan and Metal
pub trait GPUBuffer {
    /// Returns the actual allocated size (may be larger than requested due to alignment)
    fn allocated_size(&self) -> usize;

    /// Writes data to the buffer with optional offset
    /// For non-mappable buffers, this may use a staging buffer internally
    fn write_data(
        &self, data: &[u8], offset: usize,
    ) -> Result<(), GraphicsError>;

    /// Reads data from buffer (only for CPU-accessible buffers)
    fn read_data(
        &self, data: &mut [u8], offset: usize,
    ) -> Result<(), GraphicsError>;
}

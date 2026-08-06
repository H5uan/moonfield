//! wgpu buffer abstraction.

use crate::bind::BufferRef;
use crate::error::{Error, Result};
use crate::types::BufferUsage;
use crate::web::device::Device;

/// A wgpu buffer with a recorded size and a queue handle for uploads.
pub struct Buffer {
    buffer: wgpu::Buffer,
    queue: wgpu::Queue,
    size: u64,
}

impl Buffer {
    /// Create a buffer of the given size and usage.
    ///
    /// `COPY_DST` is always OR-ed into the usage so [`upload`](Self::upload)
    /// (which goes through `Queue::write_buffer`) is always legal.
    pub fn new(device: &Device, size: u64, usage: BufferUsage) -> Result<Self> {
        let buffer = device.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("moonfield-buffer"),
            size,
            usage: usage.to_wgpu() | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            buffer,
            queue: device.queue().clone(),
            size,
        })
    }

    /// Upload data to the buffer.
    ///
    /// Note: `Queue::write_buffer` requires the byte length to be a multiple
    /// of 4 (`COPY_BUFFER_ALIGNMENT`); uploads that violate this are rejected
    /// by wgpu validation.
    pub fn upload<T: Copy>(&self, data: &[T]) -> Result<()> {
        let bytes_len = std::mem::size_of_val(data);
        if bytes_len as u64 > self.size {
            return Err(Error::Validation(
                "upload data exceeds buffer size".to_string(),
            ));
        }

        // SAFETY: `data` is a valid slice of `T: Copy`, so reinterpreting its
        // bytes as `u8` covers exactly `size_of_val(data)` readable bytes.
        let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, bytes_len) };
        self.queue.write_buffer(&self.buffer, 0, bytes);
        Ok(())
    }

    /// Size of the buffer in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Access the raw `wgpu::Buffer` handle.
    pub fn raw(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

impl BufferRef for Buffer {
    fn raw_wgpu(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

//! Vulkan buffer abstraction backed by `gpu_allocator`.
//!
//! Buffers allocate through the device's shared [`Allocator`] and may live in
//! host-visible (`CpuToGpu`) or device-local (`GpuOnly`) memory. `GpuOnly`
//! buffers are uploaded via a one-shot staging copy so the RHI can hold
//! GPU-resident resources (indirect args, workgraph backing, vertex data).

use crate::bind::BufferRef;
use crate::error::{Error, Result};
use crate::types::BufferUsage;
use crate::vulkan::command::{CommandBuffer, CommandPool};
use crate::vulkan::device::Device;
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator};
use gpu_allocator::MemoryLocation;
use std::sync::{Arc, Mutex};

/// A Vulkan buffer backed by `gpu_allocator`-managed memory.
pub struct Buffer {
    buffer: vk::Buffer,
    allocation: Option<Allocation>,
    size: vk::DeviceSize,
    location: MemoryLocation,
    device: ash::Device,
    allocator: Arc<Mutex<Allocator>>,
}

impl Buffer {
    /// Create a buffer of the given size, usage, and memory location.
    ///
    /// `COPY_DST` is OR-ed into the usage so uploads always go through a
    /// staging copy on `GpuOnly` buffers (and are a no-op for host-visible
    /// ones), matching Vulkan's buffer upload convention.
    pub fn new(
        device: &Device,
        size: u64,
        usage: BufferUsage,
        location: MemoryLocation,
    ) -> Result<Self> {
        let usage = usage | BufferUsage::COPY_DST;

        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage.to_vk())
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe {
            device
                .raw()
                .create_buffer(&buffer_info, None)
                .map_err(|e| Error::Backend(format!("failed to create buffer: {:?}", e)))?
        };

        let requirements = unsafe { device.raw().get_buffer_memory_requirements(buffer) };

        let allocator = device.allocator().clone();
        let allocation = allocator
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .allocate(&AllocationCreateDesc {
                name: "buffer",
                requirements,
                location,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|e| Error::Backend(format!("failed to allocate buffer memory: {e}")))?;

        unsafe {
            device
                .raw()
                .bind_buffer_memory(buffer, allocation.memory(), allocation.offset())
                .map_err(|e| Error::Backend(format!("failed to bind buffer memory: {:?}", e)))?;
        }

        Ok(Self {
            buffer,
            allocation: Some(allocation),
            size,
            location,
            device: device.raw().clone(),
            allocator,
        })
    }

    /// Access the raw `vk::Buffer` handle.
    pub fn raw(&self) -> vk::Buffer {
        self.buffer
    }

    /// Size of the buffer in bytes.
    pub fn size(&self) -> vk::DeviceSize {
        self.size
    }

    /// The memory location this buffer was allocated in.
    pub fn location(&self) -> MemoryLocation {
        self.location
    }

    /// Upload data to the buffer.
    ///
    /// For host-visible buffers this maps and copies directly. For
    /// device-local buffers it stages through a temporary host-visible buffer
    /// and a one-shot copy command, blocking on the graphics queue until the
    /// copy completes.
    pub fn upload<T: Copy>(&self, device: &Device, data: &[T]) -> Result<()> {
        let bytes = std::mem::size_of_val(data) as vk::DeviceSize;
        if bytes > self.size {
            return Err(Error::Validation(
                "upload data exceeds buffer size".to_string(),
            ));
        }

        match self.location {
            MemoryLocation::GpuOnly => self.upload_via_staging(device, bytes, data),
            // CpuToGpu / Unknown / GpuToCpu all back host-visible memory; map
            // and copy directly. GpuToCpu is unusual for uploads but still
            // host-visible, so the same path applies.
            _ => self.upload_host_visible(bytes, data),
        }
    }

    /// Read data out of a host-visible buffer (test readback, debug dumps).
    ///
    /// Device-local (`GpuOnly`) buffers cannot be mapped; this returns an
    /// error for them.
    pub fn read<T: Copy>(&self, data: &mut [T]) -> Result<()> {
        let bytes = std::mem::size_of_val(data) as vk::DeviceSize;
        if bytes > self.size {
            return Err(Error::Validation(
                "read range exceeds buffer size".to_string(),
            ));
        }
        if self.location == MemoryLocation::GpuOnly {
            return Err(Error::Validation(
                "cannot read a device-local buffer".to_string(),
            ));
        }
        let allocation = self.allocation.as_ref().ok_or(Error::InvalidHandle)?;
        // SAFETY: gpu-allocator keeps host-visible memory blocks persistently
        // mapped, and `mapped_ptr` already points at the allocation's offset.
        // MoltenVK rejects a second vkMapMemory on the same block, so only
        // fall back to a manual map/unmap when the allocation is not mapped.
        let ptr = match allocation.mapped_ptr() {
            Some(ptr) => ptr.as_ptr(),
            None => {
                unsafe {
                    let ptr = self
                        .device
                        .map_memory(
                            allocation.memory(),
                            allocation.offset(),
                            bytes,
                            vk::MemoryMapFlags::empty(),
                        )
                        .map_err(|e| {
                            Error::Backend(format!("failed to map buffer memory: {:?}", e))
                        })?;
                    std::ptr::copy_nonoverlapping(
                        ptr as *const u8,
                        data.as_mut_ptr() as *mut u8,
                        bytes as usize,
                    );
                    self.device.unmap_memory(allocation.memory());
                }
                return Ok(());
            }
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                ptr as *const u8,
                data.as_mut_ptr() as *mut u8,
                bytes as usize,
            );
        }
        Ok(())
    }

    fn upload_host_visible<T: Copy>(&self, bytes: vk::DeviceSize, data: &[T]) -> Result<()> {
        let allocation = self.allocation.as_ref().ok_or(Error::InvalidHandle)?;
        // SAFETY: same persistent-mapping rule as `read` above — reuse the
        // allocation's mapped pointer, only manual map/unmap when absent
        // (MoltenVK rejects a second vkMapMemory on one block).
        let ptr = match allocation.mapped_ptr() {
            Some(ptr) => ptr.as_ptr(),
            None => {
                unsafe {
                    let ptr = self
                        .device
                        .map_memory(
                            allocation.memory(),
                            allocation.offset(),
                            bytes,
                            vk::MemoryMapFlags::empty(),
                        )
                        .map_err(|e| {
                            Error::Backend(format!("failed to map buffer memory: {:?}", e))
                        })?;
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr() as *const u8,
                        ptr as *mut u8,
                        bytes as usize,
                    );
                    self.device.unmap_memory(allocation.memory());
                }
                return Ok(());
            }
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr() as *const u8,
                ptr as *mut u8,
                bytes as usize,
            );
        }
        Ok(())
    }

    fn upload_via_staging<T: Copy>(
        &self,
        device: &Device,
        bytes: vk::DeviceSize,
        data: &[T],
    ) -> Result<()> {
        // Temporary host-visible staging buffer.
        let staging = Self::new(
            device,
            bytes,
            BufferUsage::COPY_SRC,
            MemoryLocation::CpuToGpu,
        )?;
        staging.upload_host_visible(bytes, data)?;

        // One-shot copy command, matching the pattern in
        // `offscreen::transition_to_shader_read`.
        let command_pool = CommandPool::new(device, device.queue_family_indices().graphics)?;
        let mut command_buffer: CommandBuffer = command_pool.allocate_command_buffer()?;

        command_buffer.begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)?;
        let copy = vk::BufferCopy::default()
            .src_offset(0)
            .dst_offset(0)
            .size(bytes);
        unsafe {
            self.device
                .cmd_copy_buffer(command_buffer.raw(), staging.buffer, self.buffer, &[copy]);
        }
        command_buffer.end()?;

        let command_buffers = [command_buffer.raw()];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        // SAFETY: the command buffer is fully recorded and the queue is valid.
        unsafe {
            device
                .raw()
                .queue_submit(
                    device.graphics_queue(),
                    std::slice::from_ref(&submit_info),
                    vk::Fence::null(),
                )
                .map_err(|e| Error::Backend(format!("failed to submit buffer copy: {:?}", e)))?;
            device
                .raw()
                .queue_wait_idle(device.graphics_queue())
                .map_err(|e| Error::Backend(format!("failed to wait for buffer copy: {:?}", e)))?;
        }
        Ok(())
    }
}

impl BufferRef for Buffer {
    fn raw_vk(&self) -> vk::Buffer {
        self.buffer
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_buffer(self.buffer, None);
        }
        if let Some(allocation) = self.allocation.take() {
            if let Err(e) = self
                .allocator
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .free(allocation)
            {
                moonfield_log::error!("failed to free buffer allocation: {e}");
            }
        }
    }
}

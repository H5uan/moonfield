//! Vulkan buffer abstraction backed by `gpu_allocator`.
//!
//! Buffers allocate through the device's shared [`Allocator`] in the crate's
//! [`Memory`] classes; [`Memory::Gpu`] buffers are uploaded via a one-shot
//! staging copy so the RHI can hold GPU-resident resources (indirect args,
//! workgraph backing, vertex data).

use crate::error::{Error, Result};
use crate::types::BufferUsage;
use crate::vulkan::device::Device;
use crate::vulkan::memory::Memory;
use crate::vulkan::retire::{RetireAction, RetirementRing};
use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator};
use std::sync::{Arc, Mutex};

/// A Vulkan buffer backed by `gpu_allocator`-managed memory.
pub struct Buffer {
    buffer: vk::Buffer,
    allocation: Option<Allocation>,
    size: u64,
    memory: Memory,
    device: ash::Device,
    allocator: Arc<Mutex<Allocator>>,
    /// Device-level retirement ring; `Drop` enqueues the teardown here.
    ring: Arc<RetirementRing>,
}

impl Buffer {
    /// Create a buffer of the given size, usage, and memory class.
    ///
    /// `COPY_DST` is OR-ed into the usage so uploads always go through a
    /// staging copy on [`Memory::Gpu`] buffers (and are a no-op for
    /// host-visible ones), matching Vulkan's buffer upload convention.
    pub fn new(device: &Device, size: u64, usage: BufferUsage, memory: Memory) -> Result<Self> {
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
                location: memory.to_location(),
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
            memory,
            device: device.raw().clone(),
            allocator,
            ring: device.retirement_ring(),
        })
    }

    /// Access the raw `vk::Buffer` handle.
    pub fn raw(&self) -> vk::Buffer {
        self.buffer
    }

    /// Size of the buffer in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The memory class this buffer was allocated in.
    pub fn memory(&self) -> Memory {
        self.memory
    }

    /// Upload data to the buffer.
    ///
    /// For host-visible buffers this maps and copies directly. For
    /// device-local buffers it stages through a temporary host-visible buffer
    /// and a one-shot copy command, blocking on the graphics queue until the
    /// copy completes.
    pub fn upload<T: Copy>(&self, device: &Device, data: &[T]) -> Result<()> {
        let bytes = std::mem::size_of_val(data) as u64;
        if bytes > self.size {
            return Err(Error::Validation(
                "upload data exceeds buffer size".to_string(),
            ));
        }

        match self.memory {
            Memory::Gpu => {
                // Stage through the device's shared frame uploader: the
                // staging region is carved from its bump arena and the copy
                // goes out as one submit+wait, with no per-call staging
                // buffer, command pool, or queue synchronization overhead.
                let uploader = device.uploader();
                let mut uploader = uploader.lock().unwrap_or_else(|e| e.into_inner());
                uploader.upload_and_wait(self, data)
            }
            // Default and Readback both back host-visible memory; map and
            // copy directly. Readback is unusual for uploads but still
            // host-visible, so the same path applies.
            _ => self.upload_host_visible(bytes, data),
        }
    }

    /// Read data out of a host-visible buffer (test readback, debug dumps).
    ///
    /// Device-local ([`Memory::Gpu`]) buffers cannot be mapped; this returns
    /// an error for them.
    pub fn read<T: Copy>(&self, data: &mut [T]) -> Result<()> {
        let bytes = std::mem::size_of_val(data) as u64;
        if bytes > self.size {
            return Err(Error::Validation(
                "read range exceeds buffer size".to_string(),
            ));
        }
        if self.memory == Memory::Gpu {
            return Err(Error::Validation(
                "cannot read a device-local buffer".to_string(),
            ));
        }
        let allocation = self.allocation.as_ref().ok_or(Error::InvalidHandle)?;
        // SAFETY: gpu-allocator keeps host-visible memory blocks persistently
        // mapped, and `mapped_ptr` already points at the allocation's offset.
        // The spec forbids vkMapMemory on an already-mapped VkDeviceMemory,
        // so only fall back to a manual map/unmap when the allocation is not
        // mapped.
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

    fn upload_host_visible<T: Copy>(&self, bytes: u64, data: &[T]) -> Result<()> {
        let allocation = self.allocation.as_ref().ok_or(Error::InvalidHandle)?;
        // SAFETY: same persistent-mapping rule as `read` above — reuse the
        // allocation's mapped pointer, only manual map/unmap when absent.
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
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // Teardown is deferred: in-flight frames may still read the buffer.
        // The ring drains RETIRE_RING frames later.
        self.ring.push(RetireAction::Buffer {
            device: self.device.clone(),
            buffer: self.buffer,
            allocation: self.allocation.take(),
            allocator: self.allocator.clone(),
        });
    }
}

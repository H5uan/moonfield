//! Vulkan buffer abstraction with host-visible memory allocation.

use crate::device::Device;
use crate::error::{Error, Result};
use crate::instance::Instance;
use ash::vk;

/// A Vulkan buffer backed by device memory.
pub struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: vk::DeviceSize,
    device: ash::Device,
}

impl Buffer {
    /// Create a buffer of the given size and usage, allocating host-visible memory.
    pub fn new(
        instance: &Instance,
        device: &Device,
        size: vk::DeviceSize,
        usage: vk::BufferUsageFlags,
    ) -> Result<Self> {
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe {
            device
                .raw()
                .create_buffer(&buffer_info, None)
                .map_err(|e| Error::Backend(format!("failed to create buffer: {:?}", e)))?
        };

        let mem_requirements = unsafe { device.raw().get_buffer_memory_requirements(buffer) };

        let memory_type_index = find_memory_type(
            instance,
            device.physical_device(),
            mem_requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_requirements.size)
            .memory_type_index(memory_type_index);

        let memory = unsafe {
            device
                .raw()
                .allocate_memory(&alloc_info, None)
                .map_err(|e| Error::Backend(format!("failed to allocate buffer memory: {:?}", e)))?
        };

        unsafe {
            device
                .raw()
                .bind_buffer_memory(buffer, memory, 0)
                .map_err(|e| Error::Backend(format!("failed to bind buffer memory: {:?}", e)))?;
        }

        Ok(Self {
            buffer,
            memory,
            size,
            device: device.raw().clone(),
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

    /// Upload data to the buffer.
    ///
    /// # Safety
    ///
    /// The buffer must be allocated with host-visible memory and the data size
    /// must not exceed the buffer size.
    pub fn upload<T: Copy>(&self, data: &[T]) -> Result<()> {
        let bytes = std::mem::size_of_val(data) as vk::DeviceSize;
        if bytes > self.size {
            return Err(Error::Validation(
                "upload data exceeds buffer size".to_string(),
            ));
        }

        unsafe {
            let ptr = self
                .device
                .map_memory(self.memory, 0, bytes, vk::MemoryMapFlags::empty())
                .map_err(|e| Error::Backend(format!("failed to map buffer memory: {:?}", e)))?;

            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr as *mut u8, bytes as usize);

            self.device.unmap_memory(self.memory);
        }

        Ok(())
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.device.free_memory(self.memory, None);
            self.device.destroy_buffer(self.buffer, None);
        }
    }
}

fn find_memory_type(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> Result<u32> {
    let mem_properties = unsafe {
        instance
            .raw()
            .get_physical_device_memory_properties(physical_device)
    };

    for i in 0..mem_properties.memory_type_count {
        let type_bits = type_filter & (1 << i);
        let supported = mem_properties.memory_types[i as usize]
            .property_flags
            .contains(properties);
        if type_bits != 0 && supported {
            return Ok(i);
        }
    }

    Err(Error::Unsupported)
}

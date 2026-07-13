//! Vulkan command pool and command buffer abstractions.

use crate::device::Device;
use crate::error::{Error, Result};
use ash::vk;

/// A Vulkan command pool.
pub struct CommandPool {
    pool: vk::CommandPool,
    device: ash::Device,
}

impl CommandPool {
    /// Create a command pool for the given queue family.
    pub fn new(device: &Device, queue_family_index: u32) -> Result<Self> {
        let create_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);

        let pool = unsafe {
            device
                .raw()
                .create_command_pool(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create command pool: {:?}", e)))?
        };

        Ok(Self {
            pool,
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::CommandPool` handle.
    pub fn raw(&self) -> vk::CommandPool {
        self.pool
    }

    /// Allocate a single primary command buffer from this pool.
    pub fn allocate_command_buffer(&self) -> Result<CommandBuffer> {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        let buffers = unsafe {
            self.device
                .allocate_command_buffers(&allocate_info)
                .map_err(|e| Error::Backend(format!("failed to allocate command buffer: {:?}", e)))?
        };

        Ok(CommandBuffer {
            buffer: buffers[0],
            pool: self.pool,
            device: self.device.clone(),
            recording: false,
        })
    }
}

impl Drop for CommandPool {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_command_pool(self.pool, None);
        }
    }
}

/// A Vulkan command buffer.
pub struct CommandBuffer {
    buffer: vk::CommandBuffer,
    pool: vk::CommandPool,
    device: ash::Device,
    recording: bool,
}

impl CommandBuffer {
    /// Access the raw `vk::CommandBuffer` handle.
    pub fn raw(&self) -> vk::CommandBuffer {
        self.buffer
    }

    /// Begin recording this command buffer.
    pub fn begin(&mut self, flags: vk::CommandBufferUsageFlags) -> Result<()> {
        let begin_info = vk::CommandBufferBeginInfo::default().flags(flags);
        unsafe {
            self.device
                .begin_command_buffer(self.buffer, &begin_info)
                .map_err(|e| Error::Backend(format!("failed to begin command buffer: {:?}", e)))?;
        }
        self.recording = true;
        Ok(())
    }

    /// End recording this command buffer.
    pub fn end(&mut self) -> Result<()> {
        unsafe {
            self.device
                .end_command_buffer(self.buffer)
                .map_err(|e| Error::Backend(format!("failed to end command buffer: {:?}", e)))?;
        }
        self.recording = false;
        Ok(())
    }

    /// Begin a render pass.
    pub fn begin_render_pass(
        &self,
        render_pass_begin_info: &vk::RenderPassBeginInfo,
        contents: vk::SubpassContents,
    ) {
        unsafe {
            self.device
                .cmd_begin_render_pass(self.buffer, render_pass_begin_info, contents);
        }
    }

    /// End the current render pass.
    pub fn end_render_pass(&self) {
        unsafe {
            self.device.cmd_end_render_pass(self.buffer);
        }
    }

    /// Bind a graphics pipeline.
    pub fn bind_graphics_pipeline(&self, pipeline: vk::Pipeline) {
        unsafe {
            self.device
                .cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
        }
    }

    /// Bind vertex buffers.
    pub fn bind_vertex_buffers(&self, first_binding: u32, buffers: &[vk::Buffer], offsets: &[vk::DeviceSize]) {
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(self.buffer, first_binding, buffers, offsets);
        }
    }

    /// Draw vertices.
    pub fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        unsafe {
            self.device.cmd_draw(
                self.buffer,
                vertex_count,
                instance_count,
                first_vertex,
                first_instance,
            );
        }
    }

    /// Insert a pipeline barrier.
    pub fn pipeline_barrier(
        &self,
        src_stage: vk::PipelineStageFlags,
        dst_stage: vk::PipelineStageFlags,
        dependency_flags: vk::DependencyFlags,
        memory_barriers: &[vk::MemoryBarrier],
        buffer_memory_barriers: &[vk::BufferMemoryBarrier],
        image_memory_barriers: &[vk::ImageMemoryBarrier],
    ) {
        unsafe {
            self.device.cmd_pipeline_barrier(
                self.buffer,
                src_stage,
                dst_stage,
                dependency_flags,
                memory_barriers,
                buffer_memory_barriers,
                image_memory_barriers,
            );
        }
    }
}

impl Drop for CommandBuffer {
    fn drop(&mut self) {
        unsafe {
            self.device
                .free_command_buffers(self.pool, std::slice::from_ref(&self.buffer));
        }
    }
}

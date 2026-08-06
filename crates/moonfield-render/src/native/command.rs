//! Vulkan command pool and command buffer abstractions.

use crate::error::{Error, Result};
use crate::native::device::Device;
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
                .map_err(|e| {
                    Error::Backend(format!("failed to allocate command buffer: {:?}", e))
                })?
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
    ///
    /// Also sets the viewport and scissor to the pass's render area —
    /// pipelines are created with dynamic viewport/scissor state.
    pub fn begin_render_pass(
        &self,
        render_pass_begin_info: &vk::RenderPassBeginInfo,
        contents: vk::SubpassContents,
    ) {
        let render_area = render_pass_begin_info.render_area;
        let viewport = vk::Viewport::default()
            .x(render_area.offset.x as f32)
            .y(render_area.offset.y as f32)
            .width(render_area.extent.width as f32)
            .height(render_area.extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        unsafe {
            self.device
                .cmd_begin_render_pass(self.buffer, render_pass_begin_info, contents);
            self.device
                .cmd_set_viewport(self.buffer, 0, std::slice::from_ref(&viewport));
            self.device
                .cmd_set_scissor(self.buffer, 0, std::slice::from_ref(&render_area));
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
    pub fn bind_vertex_buffers(
        &self,
        first_binding: u32,
        buffers: &[vk::Buffer],
        offsets: &[vk::DeviceSize],
    ) {
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(self.buffer, first_binding, buffers, offsets);
        }
    }

    /// Bind an index buffer.
    pub fn bind_index_buffer(
        &self,
        buffer: vk::Buffer,
        offset: vk::DeviceSize,
        index_type: vk::IndexType,
    ) {
        unsafe {
            self.device
                .cmd_bind_index_buffer(self.buffer, buffer, offset, index_type);
        }
    }

    /// Draw indexed primitives.
    pub fn draw_indexed(
        &self,
        index_count: u32,
        instance_count: u32,
        first_index: u32,
        vertex_offset: i32,
        first_instance: u32,
    ) {
        unsafe {
            self.device.cmd_draw_indexed(
                self.buffer,
                index_count,
                instance_count,
                first_index,
                vertex_offset,
                first_instance,
            );
        }
    }

    /// Issue `draw_count` non-indexed draws from an indirect argument buffer.
    ///
    /// `stride` is the byte stride between consecutive `DrawIndirectArgs`
    /// records and must be a multiple of 4.
    pub fn draw_indirect(
        &self,
        buffer: vk::Buffer,
        offset: vk::DeviceSize,
        draw_count: u32,
        stride: u32,
    ) {
        unsafe {
            self.device
                .cmd_draw_indirect(self.buffer, buffer, offset, draw_count, stride);
        }
    }

    /// Issue `draw_count` indexed draws from an indirect argument buffer.
    ///
    /// `stride` is the byte stride between consecutive `DrawIndexedIndirectArgs`
    /// records and must be a multiple of 4.
    pub fn draw_indexed_indirect(
        &self,
        buffer: vk::Buffer,
        offset: vk::DeviceSize,
        draw_count: u32,
        stride: u32,
    ) {
        unsafe {
            self.device
                .cmd_draw_indexed_indirect(self.buffer, buffer, offset, draw_count, stride);
        }
    }

    /// Issue non-indexed draws where the draw count is read from
    /// `count_buffer` at runtime (GPU-driven count).
    ///
    /// Requires Vulkan 1.2+ (promoted from `VK_KHR_draw_indirect_count`); the
    /// instance requests `API_VERSION_1_3` so this is always available.
    pub fn draw_indirect_count(
        &self,
        buffer: vk::Buffer,
        offset: vk::DeviceSize,
        count_buffer: vk::Buffer,
        count_buffer_offset: vk::DeviceSize,
        max_draw_count: u32,
        stride: u32,
    ) {
        unsafe {
            self.device.cmd_draw_indirect_count(
                self.buffer,
                buffer,
                offset,
                count_buffer,
                count_buffer_offset,
                max_draw_count,
                stride,
            );
        }
    }

    /// Issue indexed draws where the draw count is read from `count_buffer`
    /// at runtime (GPU-driven count).
    ///
    /// Requires Vulkan 1.2+ (promoted from `VK_KHR_draw_indirect_count`); the
    /// instance requests `API_VERSION_1_3` so this is always available.
    pub fn draw_indexed_indirect_count(
        &self,
        buffer: vk::Buffer,
        offset: vk::DeviceSize,
        count_buffer: vk::Buffer,
        count_buffer_offset: vk::DeviceSize,
        max_draw_count: u32,
        stride: u32,
    ) {
        unsafe {
            self.device.cmd_draw_indexed_indirect_count(
                self.buffer,
                buffer,
                offset,
                count_buffer,
                count_buffer_offset,
                max_draw_count,
                stride,
            );
        }
    }

    /// Draw vertices.
    pub fn draw(
        &self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
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

    /// Bind a compute pipeline.
    ///
    /// Records `vkCmdBindPipeline` with `PIPELINE_BIND_POINT_COMPUTE`. The
    /// pipeline stays bound until another `bind_compute_pipeline` call or the
    /// command buffer is reset.
    pub fn bind_compute_pipeline(&self, pipeline: vk::Pipeline) {
        unsafe {
            self.device
                .cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        }
    }

    /// Bind descriptor sets to the **graphics** bind point.
    ///
    /// `first_set` is the set number `descriptor_sets[0]` binds to; subsequent
    /// sets bind to consecutive numbers. `dynamic_offsets` supplies values
    /// for any dynamic descriptors in the bound sets (empty for static-only
    /// layouts). Use after [`bind_graphics_pipeline`](Self::bind_graphics_pipeline).
    pub fn bind_descriptor_sets(
        &self,
        pipeline_layout: vk::PipelineLayout,
        first_set: u32,
        descriptor_sets: &[vk::DescriptorSet],
        dynamic_offsets: &[u32],
    ) {
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline_layout,
                first_set,
                descriptor_sets,
                dynamic_offsets,
            );
        }
    }

    /// Bind descriptor sets to the **compute** bind point.
    ///
    /// Compute counterpart of [`bind_descriptor_sets`](Self::bind_descriptor_sets);
    /// `vkCmdBindDescriptor_sets` takes an explicit bind point, so the compute
    /// path needs its own entry point. Use after
    /// [`bind_compute_pipeline`](Self::bind_compute_pipeline).
    pub fn bind_descriptor_sets_compute(
        &self,
        pipeline_layout: vk::PipelineLayout,
        first_set: u32,
        descriptor_sets: &[vk::DescriptorSet],
        dynamic_offsets: &[u32],
    ) {
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                self.buffer,
                vk::PipelineBindPoint::COMPUTE,
                pipeline_layout,
                first_set,
                descriptor_sets,
                dynamic_offsets,
            );
        }
    }

    /// Dispatch compute workgroups.
    ///
    /// Records `vkCmdDispatch(group_count_x, group_count_y, group_count_z)`.
    /// The currently-bound compute pipeline's local workgroup size (set in the
    /// shader) multiplies these to yield total invocations.
    pub fn dispatch(&self, group_count_x: u32, group_count_y: u32, group_count_z: u32) {
        unsafe {
            self.device
                .cmd_dispatch(self.buffer, group_count_x, group_count_y, group_count_z);
        }
    }

    /// Dispatch compute workgroups with GPU-driven arguments.
    ///
    /// Reads [`DispatchIndirectArgs`](crate::DispatchIndirectArgs) (`x`, `y`,
    /// `z`) from `buffer` at `offset` at dispatch time — lets a prior compute
    /// pass decide the grid size (e.g. GPU-driven broadphase emitting a
    /// variable pair count). The buffer must have been created with
    /// [`BufferUsage::INDIRECT`](crate::BufferUsage::INDIRECT).
    pub fn dispatch_indirect(&self, buffer: vk::Buffer, offset: vk::DeviceSize) {
        unsafe {
            self.device.cmd_dispatch_indirect(self.buffer, buffer, offset);
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

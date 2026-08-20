//! Vulkan command pool and command buffer abstractions.

use crate::bindless::{GpuPtr, Stage};
use crate::error::{Error, Result};
use crate::vulkan::device::Device;
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

    /// Override the dynamic viewport (e.g. a negative height to map the
    /// engine's Y-up NDC convention onto Vulkan's top-left framebuffer
    /// origin). `begin_render_pass` resets the viewport to the render area,
    /// so call this after it.
    pub fn set_viewport(&self, viewport: vk::Viewport) {
        unsafe {
            self.device
                .cmd_set_viewport(self.buffer, 0, std::slice::from_ref(&viewport));
        }
    }

    /// Override the dynamic scissor rectangle (e.g. per-primitive clip rects
    /// in a UI renderer). `begin_render_pass` resets the scissor to the render
    /// area, so call this after it.
    pub fn set_scissor(&self, scissor: vk::Rect2D) {
        unsafe {
            self.device
                .cmd_set_scissor(self.buffer, 0, std::slice::from_ref(&scissor));
        }
    }

    /// Bind descriptor sets to the graphics bind point.
    pub fn bind_graphics_descriptor_sets(
        &self,
        layout: vk::PipelineLayout,
        first_set: u32,
        sets: &[vk::DescriptorSet],
    ) {
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                first_set,
                sets,
                &[],
            );
        }
    }

    /// Push raw bytes into the pipeline layout's push-constant block.
    pub fn push_constants(
        &self,
        layout: vk::PipelineLayout,
        stages: vk::ShaderStageFlags,
        offset: u32,
        data: &[u8],
    ) {
        unsafe {
            self.device
                .cmd_push_constants(self.buffer, layout, stages, offset, data);
        }
    }

    /// Bind a graphics pipeline.
    pub fn bind_graphics_pipeline(&self, pipeline: vk::Pipeline) {
        unsafe {
            self.device
                .cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
        }
    }

    /// Bind a compute pipeline.
    pub fn bind_compute_pipeline(&self, pipeline: vk::Pipeline) {
        unsafe {
            self.device
                .cmd_bind_pipeline(self.buffer, vk::PipelineBindPoint::COMPUTE, pipeline);
        }
    }

    /// Push the entry-point root pointers for a compute dispatch.
    ///
    /// Writes two 64-bit GPU addresses as one push-constant struct: `layout`
    /// must be the layout of the bound pipeline, whose root struct matches
    /// the kernel's pointer parameters (input @ 0, output @ 8).
    pub fn set_bindless_root(&self, layout: vk::PipelineLayout, input: GpuPtr, output: GpuPtr) {
        let root: [u64; 2] = [input.as_raw(), output.as_raw()];
        let bytes = unsafe {
            std::slice::from_raw_parts(root.as_ptr() as *const u8, std::mem::size_of_val(&root))
        };
        unsafe {
            self.device.cmd_push_constants(
                self.buffer,
                layout,
                vk::ShaderStageFlags::COMPUTE,
                0,
                bytes,
            );
        }
    }

    /// Launch a compute kernel with the given workgroup counts.
    ///
    /// Requires a bound compute pipeline and (for bindless kernels) a root
    /// pointer set via [`set_bindless_root`] ahead of the call.
    pub fn dispatch(&self, x: u32, y: u32, z: u32) {
        unsafe { self.device.cmd_dispatch(self.buffer, x, y, z) };
    }

    /// Order the end of `before` against the start of `after` without naming
    /// any resource.
    ///
    /// Emits a single global memory barrier (sync2): all memory writes by
    /// `before` become visible to all memory accesses in `after`. This is the
    /// bindless form of synchronization — shaders touch memory through
    /// pointers, so a resource list would be both impossible and meaningless.
    pub fn barrier(&self, before: Stage, after: Stage) {
        let memory_barrier = vk::MemoryBarrier2::default()
            .src_stage_mask(before.to_vk())
            .src_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(after.to_vk())
            .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE);
        let dependency_info =
            vk::DependencyInfo::default().memory_barriers(std::slice::from_ref(&memory_barrier));
        unsafe {
            self.device
                .cmd_pipeline_barrier2(self.buffer, &dependency_info);
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

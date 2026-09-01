//! Vulkan command pool and command buffer abstractions.

use crate::bind::{BindGroup, TextureView};
use crate::bindless::{BarrierHazard, GpuAllocation, GpuPtr, Stage};
use crate::error::{Error, Result};
use crate::types::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CompareOp, CullMode, FrontFace, LoadOp,
    Rect2d, ShaderStages, StoreOp, Viewport,
};
use crate::vulkan::device::Device;
use crate::{BlendMode, Buffer, GraphicsPipeline, IndexFormat, PipelineLayout};
use ash::vk;
use std::sync::Arc;

/// One attachment of a render pass, in the crate's own vocabulary.
pub struct RenderAttachment {
    /// The image view rendered into.
    pub view: TextureView,
    /// The layout the image is in during the pass (and stays in).
    pub layout: AttachmentLayout,
    /// Load behavior at pass begin.
    pub load: LoadOp,
    /// Store behavior at pass end.
    pub store: StoreOp,
    /// Clear value used when `load` is [`LoadOp::Clear`].
    pub clear: ClearValue,
}

/// A render pass description for dynamic rendering.
pub struct RenderPassDesc<'a> {
    /// The pixel area rendered into; also sets the initial viewport/scissor.
    pub render_area: Rect2d,
    /// Number of array layers rendered.
    pub layer_count: u32,
    /// Color attachments.
    pub color_attachments: &'a [RenderAttachment],
    /// Optional depth attachment.
    pub depth_attachment: Option<RenderAttachment>,
}

/// A Vulkan command pool.
pub struct CommandPool {
    pool: vk::CommandPool,
    device: ash::Device,
    /// Shared aggregated device-extension loaders (an `Arc`, so command
    /// buffers from this pool share the same function-pointer tables).
    ext: Arc<crate::vulkan::DeviceExtensionFunctions>,
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
            ext: device.extension_fns(),
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
            ext: self.ext.clone(),
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

/// Depth testing state, set per draw via dynamic state (Vulkan 1.3 core).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthState {
    pub test_enable: bool,
    pub write_enable: bool,
    pub compare_op: CompareOp,
}

/// Rasterizer cull state, set per draw via dynamic state (Vulkan 1.3 core).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CullState {
    pub cull_mode: CullMode,
    pub front_face: FrontFace,
}

/// A Vulkan command buffer.
pub struct CommandBuffer {
    buffer: vk::CommandBuffer,
    pool: vk::CommandPool,
    device: ash::Device,
    /// Shared aggregated device-extension loaders (`Arc<...>`, so every
    /// command buffer from a pool shares the same function-pointer tables —
    /// wgpu keeps the table in `Arc<DeviceShared>` and never copies it into
    /// the command buffer either).
    ext: Arc<crate::vulkan::DeviceExtensionFunctions>,
    recording: bool,
}

impl CommandBuffer {
    /// Access the raw `vk::CommandBuffer` handle.
    pub fn raw(&self) -> vk::CommandBuffer {
        self.buffer
    }

    /// Begin recording this command buffer.
    pub fn begin(&mut self, usage: CommandBufferUsage) -> Result<()> {
        let begin_info = vk::CommandBufferBeginInfo::default().flags(usage.to_vk());
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
    pub fn begin_rendering(&self, desc: &RenderPassDesc) {
        let color_attachments: Vec<vk::RenderingAttachmentInfo> = desc
            .color_attachments
            .iter()
            .map(|att| {
                vk::RenderingAttachmentInfo::default()
                    .image_view(att.view.raw_vk())
                    .image_layout(att.layout.to_vk())
                    .load_op(match att.load {
                        LoadOp::Load => vk::AttachmentLoadOp::LOAD,
                        LoadOp::Clear => vk::AttachmentLoadOp::CLEAR,
                    })
                    .store_op(match att.store {
                        StoreOp::Store => vk::AttachmentStoreOp::STORE,
                        StoreOp::Discard => vk::AttachmentStoreOp::DONT_CARE,
                    })
                    .clear_value(att.clear.to_vk())
            })
            .collect();
        let depth_attachment = desc.depth_attachment.as_ref().map(|att| {
            vk::RenderingAttachmentInfo::default()
                .image_view(att.view.raw_vk())
                .image_layout(att.layout.to_vk())
                .load_op(match att.load {
                    LoadOp::Load => vk::AttachmentLoadOp::LOAD,
                    LoadOp::Clear => vk::AttachmentLoadOp::CLEAR,
                })
                .store_op(match att.store {
                    StoreOp::Store => vk::AttachmentStoreOp::STORE,
                    StoreOp::Discard => vk::AttachmentStoreOp::DONT_CARE,
                })
                .clear_value(att.clear.to_vk())
        });
        let render_area = desc.render_area.to_vk();
        let mut rendering_info = vk::RenderingInfo::default()
            .render_area(render_area)
            .layer_count(desc.layer_count)
            .color_attachments(&color_attachments);
        if let Some(att) = depth_attachment.as_ref() {
            rendering_info = rendering_info.depth_attachment(att);
        }
        let viewport = vk::Viewport::default()
            .x(render_area.offset.x as f32)
            .y(render_area.offset.y as f32)
            .width(render_area.extent.width as f32)
            .height(render_area.extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        unsafe {
            self.device
                .cmd_begin_rendering(self.buffer, &rendering_info);
            self.device
                .cmd_set_viewport(self.buffer, 0, std::slice::from_ref(&viewport));
            self.device
                .cmd_set_scissor(self.buffer, 0, std::slice::from_ref(&render_area));
            // Dynamic states are sticky across passes, so entering a rendering
            // pass resets them to defaults (no_gfx_api convention): blend off,
            // back-face culling, depth off. Draws set only the differences.
            self.device
                .cmd_set_cull_mode(self.buffer, vk::CullModeFlags::BACK);
            self.device
                .cmd_set_front_face(self.buffer, vk::FrontFace::CLOCKWISE);
            self.device.cmd_set_depth_test_enable(self.buffer, false);
            self.device.cmd_set_depth_write_enable(self.buffer, false);
            self.device
                .cmd_set_depth_compare_op(self.buffer, vk::CompareOp::GREATER_OR_EQUAL);
            self.ext
                .extended_dynamic_state3
                .cmd_set_color_blend_enable(self.buffer, 0, &[0]);
            self.ext.extended_dynamic_state3.cmd_set_color_write_mask(
                self.buffer,
                0,
                &[vk::ColorComponentFlags::RGBA],
            );
        }
    }

    /// End the current render pass.
    pub fn end_rendering(&self) {
        unsafe { self.device.cmd_end_rendering(self.buffer) }
    }

    /// Override the dynamic viewport (e.g. a negative height to map the
    /// engine's Y-up NDC convention onto Vulkan's top-left framebuffer
    /// origin — see [`Viewport::y_flipped`]). `begin_rendering` resets the
    /// viewport to the render area, so call this after it.
    pub fn set_viewport(&self, viewport: Viewport) {
        unsafe {
            self.device
                .cmd_set_viewport(self.buffer, 0, std::slice::from_ref(&viewport.to_vk()));
        }
    }

    /// Override the dynamic scissor rectangle (e.g. per-primitive clip rects
    /// in a UI pass). `begin_rendering` resets the scissor to the render
    /// area, so call this after it.
    pub fn set_scissor(&self, scissor: Rect2d) {
        unsafe {
            self.device
                .cmd_set_scissor(self.buffer, 0, std::slice::from_ref(&scissor.to_vk()));
        }
    }

    /// Set the dynamic color blend state for the current draw (requires
    /// `VK_EXT_extended_dynamic_state3`, enabled at device creation). Resets
    /// to blend-disabled in [`begin_rendering`](Self::begin_rendering).
    pub fn set_blend_state(&self, blend: BlendMode) {
        let enable = matches!(blend, BlendMode::PremultipliedAlpha);
        // SAFETY: the command buffer is recording and the attachment indices
        // target attachment 0 of the current dynamic rendering pass.
        unsafe {
            self.ext.extended_dynamic_state3.cmd_set_color_blend_enable(
                self.buffer,
                0,
                &[enable as u32],
            );
            if enable {
                self.ext
                    .extended_dynamic_state3
                    .cmd_set_color_blend_equation(
                        self.buffer,
                        0,
                        &[vk::ColorBlendEquationEXT {
                            src_color_blend_factor: vk::BlendFactor::ONE,
                            dst_color_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
                            color_blend_op: vk::BlendOp::ADD,
                            src_alpha_blend_factor: vk::BlendFactor::ONE_MINUS_DST_ALPHA,
                            dst_alpha_blend_factor: vk::BlendFactor::ONE,
                            alpha_blend_op: vk::BlendOp::ADD,
                        }],
                    );
            }
            self.ext.extended_dynamic_state3.cmd_set_color_write_mask(
                self.buffer,
                0,
                &[vk::ColorComponentFlags::RGBA],
            );
        }
    }

    /// Set the dynamic rasterizer cull state (Vulkan 1.3 core). The caller
    /// picks both the cull mode and the front-face orientation; the engine's
    /// Y-flip viewport pairs with `FrontFace::CLOCKWISE`.
    pub fn set_cull_state(&self, state: CullState) {
        // SAFETY: the command buffer is recording and the pipeline uses
        // dynamic cull/front-face state.
        unsafe {
            self.device
                .cmd_set_cull_mode(self.buffer, state.cull_mode.to_vk());
            self.device
                .cmd_set_front_face(self.buffer, state.front_face.to_vk());
        }
    }

    /// Set the dynamic depth-testing state (Vulkan 1.3 core).
    pub fn set_depth_state(&self, state: DepthState) {
        // SAFETY: the command buffer is recording and the pipeline uses
        // dynamic depth-test/write/compare state.
        unsafe {
            self.device
                .cmd_set_depth_test_enable(self.buffer, state.test_enable);
            self.device
                .cmd_set_depth_write_enable(self.buffer, state.write_enable);
            self.device
                .cmd_set_depth_compare_op(self.buffer, state.compare_op.to_vk());
        }
    }

    /// Bind descriptor sets to the graphics bind point.
    pub fn bind_graphics_descriptor_sets(
        &self,
        layout: PipelineLayout,
        first_set: u32,
        sets: &[&BindGroup],
    ) {
        let raw: Vec<vk::DescriptorSet> = sets.iter().map(|set| set.raw_vk()).collect();
        unsafe {
            self.device.cmd_bind_descriptor_sets(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                layout.to_vk(),
                first_set,
                &raw,
                &[],
            );
        }
    }

    /// Push raw bytes into the pipeline layout's push-constant block.
    pub fn push_constants(
        &self,
        layout: PipelineLayout,
        stages: ShaderStages,
        offset: u32,
        data: &[u8],
    ) {
        unsafe {
            self.device.cmd_push_constants(
                self.buffer,
                layout.to_vk(),
                stages.to_vk(),
                offset,
                data,
            );
        }
    }

    /// Update push data — the extension's push-constant storage class,
    /// available to all shader stages. This is the fast path for root data
    /// that does not fit inline: store device addresses of per-draw structs,
    /// like a larger push constant block. Recording push data invalidates
    /// non-heap descriptor state and vice versa, so it pairs with pure-heap
    /// pipelines. The total written is bounded by `max_push_data_size` at
    /// record time (validation flags overruns).
    pub fn push_data(&self, offset: u32, data: &[u8]) {
        let range = vk::HostAddressRangeConstEXT {
            address: data.as_ptr().cast(),
            size: data.len(),
            _marker: std::marker::PhantomData,
        };
        let info = vk::PushDataInfoEXT::default().offset(offset).data(range);
        // SAFETY: the byte range is valid for the call and the command buffer is
        // recording.
        unsafe {
            self.ext.descriptor_heap.cmd_push_data(self.buffer, &info);
        }
    }

    /// Bind a graphics pipeline.
    pub fn bind_graphics_pipeline(&self, pipeline: &GraphicsPipeline) {
        unsafe {
            self.device.cmd_bind_pipeline(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.raw(),
            );
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

    /// Launch a compute kernel whose workgroup counts are read from GPU memory.
    pub fn dispatch_indirect(&self, args: &GpuAllocation) {
        unsafe {
            self.device
                .cmd_dispatch_indirect(self.buffer, args.buffer(), 0);
        }
    }

    pub fn cmd_memcpy(&self, dst: &GpuAllocation, src: &GpuAllocation, size: u64) {
        let region = vk::BufferCopy2::default()
            .src_offset(0)
            .dst_offset(0)
            .size(size);
        let copy_info = vk::CopyBufferInfo2::default()
            .src_buffer(src.buffer())
            .dst_buffer(dst.buffer())
            .regions(std::slice::from_ref(&region));
        unsafe {
            self.device.cmd_copy_buffer2(self.buffer, &copy_info);
        }
    }

    /// Order the end of `before` against the start of `after` without naming
    /// any resource.
    ///
    /// Emits a single global memory barrier (sync2): all memory writes by
    /// `before` become visible to all memory accesses in `after`. This is the
    /// bindless form of synchronization — shaders touch memory through
    /// pointers, so a resource list would be both impossible and meaningless.
    pub fn barrier(&self, before: Stage, after: Stage, hazard: BarrierHazard) {
        let (src_access, dst_access) = match hazard {
            BarrierHazard::Memory => (
                vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
                vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
            ),
            BarrierHazard::Descriptors => (
                vk::AccessFlags2::MEMORY_WRITE | vk::AccessFlags2::MEMORY_READ,
                vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::SHADER_SAMPLED_READ,
            ),
        };
        let memory_barrier = vk::MemoryBarrier2::default()
            .src_stage_mask(before.to_vk())
            .src_access_mask(src_access)
            .dst_stage_mask(after.to_vk())
            .dst_access_mask(dst_access);
        let dependency_info =
            vk::DependencyInfo::default().memory_barriers(std::slice::from_ref(&memory_barrier));
        unsafe {
            self.device
                .cmd_pipeline_barrier2(self.buffer, &dependency_info);
        }
    }

    /// Bind vertex buffers.
    pub fn bind_vertex_buffers(&self, first_binding: u32, buffers: &[&Buffer], offsets: &[u64]) {
        let raw: Vec<vk::Buffer> = buffers.iter().map(|buffer| buffer.raw()).collect();
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(self.buffer, first_binding, &raw, offsets);
        }
    }

    /// Bind an index buffer.
    pub fn bind_index_buffer(&self, buffer: &Buffer, offset: u64, format: IndexFormat) {
        unsafe {
            self.device
                .cmd_bind_index_buffer(self.buffer, buffer.raw(), offset, format.to_vk());
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

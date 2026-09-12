//! Vulkan command pool and command buffer abstractions.

use crate::error::{Error, Result};
use crate::types::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CompareOp, CullMode, FrontFace, LoadOp,
    Rect2d, StoreOp, Viewport,
};
use crate::vulkan::device::Device;
use crate::vulkan::memory::GpuPtr;
use crate::vulkan::sync::{Access, Stage, TimestampQueryPool};
use crate::vulkan::view::TextureView;
use crate::{BlendMode, ComputePipeline, GraphicsPipeline};
use ash::vk;
use std::sync::Arc;

/// One attachment of a render pass, in the crate's own vocabulary.
#[derive(Clone)]
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

        // SAFETY: the device is valid and the create info names an existing
        // queue family.
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

    /// Allocate a single primary command buffer from this pool.
    pub fn allocate_command_buffer(&self) -> Result<CommandBuffer> {
        let allocate_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        // SAFETY: the pool is valid and the allocate info requests primary
        // buffers from it.
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
        // SAFETY: the pool was created by this device and is destroyed exactly
        // once, here.
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
    pub(crate) fn raw(&self) -> vk::CommandBuffer {
        self.buffer
    }

    /// Begin recording this command buffer.
    pub fn begin(&mut self, usage: CommandBufferUsage) -> Result<()> {
        let begin_info = vk::CommandBufferBeginInfo::default().flags(usage.to_vk());
        // SAFETY: the command buffer is allocated and not pending execution —
        // owners re-record only after the buffer's frame slot is reacquired
        // (the uploader waits on its slot timeline), and the pool was created
        // with RESET_COMMAND_BUFFER.
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
        // SAFETY: the command buffer is in the recording state — callers pair
        // `end` with a successful `begin`.
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
        // SAFETY: the command buffer is recording, the attachment views are
        // live and in the layouts declared by `desc`, and pipelines used in
        // the pass declare these dynamic states (see `pipeline.rs`).
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
        // SAFETY: the command buffer is inside a render pass begun by
        // `begin_rendering`.
        unsafe { self.device.cmd_end_rendering(self.buffer) }
    }

    /// Override the dynamic viewport (e.g. a negative height to map the
    /// engine's Y-up NDC convention onto Vulkan's top-left framebuffer
    /// origin — see [`Viewport::y_flipped`]). `begin_rendering` resets the
    /// viewport to the render area, so call this after it.
    pub fn set_viewport(&self, viewport: Viewport) {
        // SAFETY: the command buffer is recording and pipelines declare
        // dynamic viewport state.
        unsafe {
            self.device
                .cmd_set_viewport(self.buffer, 0, std::slice::from_ref(&viewport.to_vk()));
        }
    }

    /// Override the dynamic scissor rectangle (e.g. per-primitive clip rects
    /// in a UI pass). `begin_rendering` resets the scissor to the render
    /// area, so call this after it.
    pub fn set_scissor(&self, scissor: Rect2d) {
        // SAFETY: the command buffer is recording and pipelines declare
        // dynamic scissor state.
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

    /// Update push data — the root-data interface of descriptor-heap
    /// pipelines, delivered to shaders through the existing push-constant
    /// storage class. Range updates are offset-addressed (4-byte aligned);
    /// bytes outside the written range keep their previous values for
    /// subsequent commands (GPU-verified by `command_push_data`). Push
    /// constants rely on set layout state and are incompatible with
    /// descriptor-heap pipelines, and every pipeline is one, so this is the
    /// only root-data path. The total written is bounded by
    /// `max_push_data_size` at record time (validation flags overruns).
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
        // SAFETY: the command buffer is recording and the pipeline is a live
        // graphics pipeline.
        unsafe {
            self.device.cmd_bind_pipeline(
                self.buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.raw(),
            );
        }
    }

    /// Bind a compute pipeline.
    pub fn bind_compute_pipeline(&self, pipeline: &ComputePipeline) {
        // SAFETY: the command buffer is recording and the pipeline is a live
        // compute pipeline.
        unsafe {
            self.device.cmd_bind_pipeline(
                self.buffer,
                vk::PipelineBindPoint::COMPUTE,
                pipeline.raw(),
            );
        }
    }

    /// Push the entry-point root pointers for a compute dispatch.
    ///
    /// Writes two 64-bit GPU addresses as one push-data block (16 bytes),
    /// matching the kernel's pointer parameters (input @ 0, output @ 8) read
    /// from the push-constant storage class.
    pub fn set_bindless_root(&self, input: GpuPtr, output: GpuPtr) {
        let root: [u64; 2] = [input.as_raw(), output.as_raw()];
        // SAFETY: `root` is a stack array; the byte view is valid for the call.
        let bytes = unsafe {
            std::slice::from_raw_parts(root.as_ptr() as *const u8, std::mem::size_of_val(&root))
        };
        self.push_data(0, bytes);
    }

    /// Launch a compute kernel with the given workgroup counts.
    ///
    /// Requires a bound compute pipeline and (for bindless kernels) a root
    /// pointer set via [`set_bindless_root`] ahead of the call.
    pub fn dispatch(&self, x: u32, y: u32, z: u32) {
        // SAFETY: the command buffer is recording with a compute pipeline
        // bound and its root data pushed (caller contract; see the method
        // docs).
        unsafe { self.device.cmd_dispatch(self.buffer, x, y, z) };
    }

    /// Launch a compute kernel whose workgroup counts are read from GPU
    /// memory at `args` (a `DispatchIndirectArgs` record).
    ///
    /// Address-based (`vkCmdDispatchIndirect2KHR`): the argument location is
    /// a device address, so no buffer handle or offset is involved.
    pub fn dispatch_indirect(&self, args: GpuPtr) {
        let info = vk::DispatchIndirect2InfoKHR::default()
            .address_range(
                vk::DeviceAddressRangeKHR::default()
                    .address(args.as_raw())
                    .size(std::mem::size_of::<crate::DispatchIndirectArgs>() as u64),
            )
            .address_flags(vk::AddressCommandFlagsKHR::FULLY_BOUND);
        // SAFETY: the command buffer is recording with a compute pipeline
        // bound, and `args` addresses a live, fully-bound allocation holding
        // a dispatch record (caller contract).
        unsafe {
            self.ext
                .device_address_commands
                .cmd_dispatch_indirect2(self.buffer, &info);
        }
    }

    /// Copy `size` GPU bytes from `src` to `dst` — both device addresses
    /// (`vkCmdCopyMemoryKHR`), with no buffer handles involved.
    pub fn cmd_memcpy(&self, dst: GpuPtr, src: GpuPtr, size: u64) {
        let region = vk::DeviceMemoryCopyKHR::default()
            .src_range(
                vk::DeviceAddressRangeKHR::default()
                    .address(src.as_raw())
                    .size(size),
            )
            .src_flags(vk::AddressCommandFlagsKHR::FULLY_BOUND)
            .dst_range(
                vk::DeviceAddressRangeKHR::default()
                    .address(dst.as_raw())
                    .size(size),
            )
            .dst_flags(vk::AddressCommandFlagsKHR::FULLY_BOUND);
        let copy_info =
            vk::CopyDeviceMemoryInfoKHR::default().regions(std::slice::from_ref(&region));
        // SAFETY: the command buffer is recording and both address ranges
        // reference live, fully-bound, transfer-capable allocations whose
        // `size` ranges fit (caller contract).
        unsafe {
            self.ext
                .device_address_commands
                .cmd_copy_memory(self.buffer, &copy_info);
        }
    }

    /// Reset every query in `queries` (core `vkCmdResetQueryPool`). Record
    /// it before a recording pass's [`write_timestamp`](Self::write_timestamp)
    /// calls whenever the pool is reused across submissions.
    pub fn reset_timestamps(&self, queries: &TimestampQueryPool) {
        // SAFETY: the command buffer is recording and the pool is live; the
        // caller records this before the pass's timestamp writes.
        unsafe {
            self.device
                .cmd_reset_query_pool(self.buffer, queries.raw(), 0, queries.count());
        }
    }

    /// Write a GPU timestamp into query `index` of `queries`, stamped at
    /// `stage` (sync2 `vkCmdWriteTimestamp2`). Pass a single stage —
    /// multi-stage masks stamp at an unspecified one of them.
    pub fn write_timestamp(&self, queries: &TimestampQueryPool, index: u32, stage: Stage) {
        // SAFETY: the command buffer is recording, the pool is live, and
        // `index` is in range and reset for this submission (caller
        // contract; see `reset_timestamps`).
        unsafe {
            self.device
                .cmd_write_timestamp2(self.buffer, stage.to_vk(), queries.raw(), index);
        }
    }

    /// Resolve `count` timestamps starting at query `first` into `count`
    /// consecutive `u64` tick values at the device address `dst`
    /// (`vkCmdCopyQueryPoolResultsToMemoryKHR`, 64-bit results with `WAIT`).
    /// The consumer reads them from the allocation's host mapping after the
    /// submission's timeline point; convert with
    /// [`TimestampQueryPool::timestamp_period_ns`].
    pub fn resolve_timestamps(
        &self,
        queries: &TimestampQueryPool,
        first: u32,
        count: u32,
        dst: GpuPtr,
    ) {
        let dst_range = vk::StridedDeviceAddressRangeKHR::default()
            .address(dst.as_raw())
            .size(count as u64 * 8)
            .stride(8);
        // SAFETY: the command buffer is recording, the pool is live, the
        // query range was written this submission, and `dst` addresses a
        // live, fully-bound allocation with room for `count` u64s (caller
        // contract).
        unsafe {
            self.ext
                .device_address_commands
                .cmd_copy_query_pool_results_to_memory(
                    self.buffer,
                    queries.raw(),
                    first,
                    count,
                    &dst_range,
                    vk::AddressCommandFlagsKHR::FULLY_BOUND,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
                );
        }
    }

    /// Order the end of `before` against the start of `after` without naming
    /// any resource.
    ///
    /// Emits a single global memory barrier (sync2): the `before_access`
    /// writes become visible to the `after_access` accesses. This is the
    /// bindless form of synchronization — shaders touch memory through
    /// pointers, so a resource list would be both impossible and meaningless.
    /// Keep the access masks honest: name the accesses each side actually
    /// performs (e.g. heap-sampled reads need [`Access::SHADER_SAMPLED_READ`]
    /// on the destination side), and widen the *stage* rather than the access
    /// when in doubt.
    pub fn barrier(
        &self,
        before: Stage,
        before_access: Access,
        after: Stage,
        after_access: Access,
    ) {
        let memory_barrier = vk::MemoryBarrier2::default()
            .src_stage_mask(before.to_vk())
            .src_access_mask(before_access.to_vk())
            .dst_stage_mask(after.to_vk())
            .dst_access_mask(after_access.to_vk());
        let dependency_info =
            vk::DependencyInfo::default().memory_barriers(std::slice::from_ref(&memory_barrier));
        // SAFETY: the command buffer is recording; a global memory barrier
        // names no resources, so no object lifetimes are involved.
        unsafe {
            self.device
                .cmd_pipeline_barrier2(self.buffer, &dependency_info);
        }
    }

    /// Issue `draw_count` non-indexed draws with argument records read from
    /// `args` (a device address — `vkCmdDrawIndirect2KHR`).
    ///
    /// `stride` is the byte stride between consecutive `DrawIndirectArgs`
    /// records and must be a multiple of 4. Mid-buffer starts use
    /// [`GpuPtr::offset`]; there is no offset parameter.
    pub fn draw_indirect(&self, args: GpuPtr, draw_count: u32, stride: u32) {
        let info = vk::DrawIndirect2InfoKHR::default()
            .address_range(
                vk::StridedDeviceAddressRangeKHR::default()
                    .address(args.as_raw())
                    .size(draw_count as u64 * stride as u64)
                    .stride(stride as u64),
            )
            .address_flags(vk::AddressCommandFlagsKHR::FULLY_BOUND)
            .draw_count(draw_count);
        // SAFETY: the command buffer is inside a render pass with a graphics
        // pipeline bound, and `args` addresses a live, fully-bound allocation
        // holding `draw_count` stride-spaced argument records (caller
        // contract).
        unsafe {
            self.ext
                .device_address_commands
                .cmd_draw_indirect2(self.buffer, &info);
        }
    }

    /// Issue non-indexed draws where the draw count is read from
    /// `count` at runtime (GPU-driven count) — `vkCmdDrawIndirectCount2KHR`,
    /// both arguments and count addressed by `GpuPtr`.
    pub fn draw_indirect_count(
        &self,
        args: GpuPtr,
        count: GpuPtr,
        max_draw_count: u32,
        stride: u32,
    ) {
        let info = vk::DrawIndirectCount2InfoKHR::default()
            .address_range(
                vk::StridedDeviceAddressRangeKHR::default()
                    .address(args.as_raw())
                    .size(max_draw_count as u64 * stride as u64)
                    .stride(stride as u64),
            )
            .address_flags(vk::AddressCommandFlagsKHR::FULLY_BOUND)
            .count_address_range(
                vk::DeviceAddressRangeKHR::default()
                    .address(count.as_raw())
                    .size(4),
            )
            .count_address_flags(vk::AddressCommandFlagsKHR::FULLY_BOUND)
            .max_draw_count(max_draw_count);
        // SAFETY: the command buffer is inside a render pass with a graphics
        // pipeline bound, and both addresses reference live, fully-bound
        // allocations holding the argument records and the u32 count (caller
        // contract).
        unsafe {
            self.ext
                .device_address_commands
                .cmd_draw_indirect_count2(self.buffer, &info);
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
        // SAFETY: the command buffer is inside a render pass with a graphics
        // pipeline bound.
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

    /// Insert sync2 image-memory barriers (layout transitions and
    /// image-access ordering).
    ///
    /// Crate-internal: the upload and offscreen paths' image-layout
    /// transitions use it; resource-less synchronization uses
    /// [`barrier`](Self::barrier). Callers build the
    /// [`vk::ImageMemoryBarrier2`] structs, so stage/access masks and layouts
    /// are spelled out at the call site.
    pub(crate) fn image_barriers(&self, barriers: &[vk::ImageMemoryBarrier2]) {
        let dependency_info = vk::DependencyInfo::default().image_memory_barriers(barriers);
        // SAFETY: the command buffer is recording and the barrier structs
        // reference live images (upload/offscreen layout transitions).
        unsafe {
            self.device
                .cmd_pipeline_barrier2(self.buffer, &dependency_info);
        }
    }
}

impl Drop for CommandBuffer {
    fn drop(&mut self) {
        // SAFETY: the buffer was allocated from `pool` and is freed exactly
        // once here; owners keep the pool alive past their buffers (see
        // `FrameUploader`'s field drop order).
        unsafe {
            self.device
                .free_command_buffers(self.pool, std::slice::from_ref(&self.buffer));
        }
    }
}

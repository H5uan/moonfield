//! Command pool and command buffer: `MTL4CommandBuffer` recording with the
//! per-buffer `MTL4ArgumentTable`.
//!
//! The recording surface matches the engine layer's vocabulary: render and
//! compute passes, dynamic viewport/scissor/cull state, root data via
//! [`push_data`](CommandBuffer::push_data) — the Metal counterpart of the
//! Vulkan push-data root blob — and stage barriers.

use moonfield_rhi_core::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CompareOp, CullMode, FrontFace, LoadOp,
    Rect2d, StoreOp, Viewport,
};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSUInteger;
use objc2_metal::{
    MTL4ArgumentTable, MTL4ArgumentTableDescriptor, MTL4CommandBuffer, MTL4CommandEncoder,
    MTL4ComputeCommandEncoder, MTL4RenderCommandEncoder, MTL4RenderPassDescriptor, MTLDevice,
    MTLPrimitiveType, MTLRenderStages, MTLSize, MTLViewport,
};

use crate::device::{Device, DeviceContext};
use crate::formats::ToMetal;
use crate::memory::{GpuAllocation, GpuPtr, Memory};
use crate::pipeline::{BlendMode, ComputePipeline, GraphicsPipeline};
use crate::sync::{Access, Stage};
use crate::view::TextureView;

/// Size of the per-command-buffer root-blob ring.
const ROOT_RING_SIZE: u64 = 64 * 1024;

/// Depth test state, set per draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthState {
    pub test_enable: bool,
    pub write_enable: bool,
    pub compare_op: CompareOp,
}

/// Rasterizer cull state, set per draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CullState {
    pub cull_mode: CullMode,
    pub front_face: FrontFace,
}

/// The render-pass context an encoder rebuild needs: the attachment, the
/// store action, and the render area (a rebuild always loads).
struct PassRebuild {
    view: TextureView,
    store: StoreOp,
    clear: ClearValue,
    render_area: Rect2d,
}

/// One color/depth attachment of a render pass (the Metal counterpart of the
/// Vulkan backend's `RenderAttachment`).
pub struct RenderAttachment {
    /// The image rendered into.
    pub view: TextureView,
    /// The layout role during the pass. Metal has no image layouts; the
    /// field exists so callers keep one pass-description shape across
    /// backends.
    pub layout: AttachmentLayout,
    /// Load behavior at pass begin.
    pub load: LoadOp,
    /// Store behavior at pass end.
    pub store: StoreOp,
    /// Clear value used when `load` is [`LoadOp::Clear`].
    pub clear: ClearValue,
}

/// A render-pass description (single target in this skeleton).
pub struct RenderPassDesc<'a> {
    /// The pixel area rendered into; also sets the viewport.
    pub render_area: Rect2d,
    /// Color attachments; the Metal backend uses the first one.
    pub color_attachments: &'a [RenderAttachment],
    /// Optional depth attachment. The Metal backend ignores it.
    pub depth_attachment: Option<RenderAttachment>,
}

/// Allocates command buffers on the device's command allocator.
#[derive(Clone)]
pub struct CommandPool {
    ctx: DeviceContext,
}

impl CommandPool {
    pub fn new(device: &Device) -> Self {
        Self {
            ctx: device.ctx().clone(),
        }
    }

    /// Begin a fresh command buffer with its own argument table.
    pub fn allocate(&self) -> CommandBuffer {
        let shared = self.ctx.shared();
        let cmdbuf = shared
            .device()
            .newCommandBuffer()
            .expect("newCommandBuffer failed");
        cmdbuf.beginCommandBufferWithAllocator(shared.command_allocator());

        let table_descriptor = MTL4ArgumentTableDescriptor::new();
        table_descriptor.setMaxBufferBindCount(8);
        let argument_table = shared
            .device()
            .newArgumentTableWithDescriptor_error(&table_descriptor)
            .expect("newArgumentTable failed");

        CommandBuffer {
            device: self.ctx.device(),
            cmdbuf,
            argument_table,
            encoder: None,
            compute_encoder: None,
            threads_per_group: None,
            replay_compute: None,
            replay_graphics: None,
            replay_viewport: None,
            pass_rebuild: None,
            encoded: false,
            root_staging: Vec::new(),
            root_ring: None,
            root_cursor: 0,
            ended: false,
        }
    }
}

/// An in-recording `MTL4CommandBuffer` with its per-buffer argument table
/// and root-blob ring.
///
/// Root data (`push_data`) snapshots into a private ring and rebinds
/// argument-table slot 0 on every push: each draw/dispatch reads the blob
/// as it was at the last push, matching Vulkan push-data semantics.
pub struct CommandBuffer {
    device: Device,
    cmdbuf: Retained<ProtocolObject<dyn MTL4CommandBuffer>>,
    argument_table: Retained<ProtocolObject<dyn MTL4ArgumentTable>>,
    encoder: Option<Retained<ProtocolObject<dyn MTL4RenderCommandEncoder>>>,
    compute_encoder: Option<Retained<ProtocolObject<dyn MTL4ComputeCommandEncoder>>>,
    /// Threads per threadgroup recorded by the last `bind_pipeline`.
    threads_per_group: Option<MTLSize>,
    /// The compute pipeline to replay after an encoder rebuild.
    replay_compute: Option<ComputePipeline>,
    /// The graphics pipeline to replay after an encoder rebuild.
    replay_graphics: Option<GraphicsPipeline>,
    /// The viewport to replay after a render-encoder rebuild.
    replay_viewport: Option<Viewport>,
    /// The render-pass context for encoder rebuilds.
    pass_rebuild: Option<PassRebuild>,
    /// Whether the active encoder already encoded commands (an encoder
    /// fixes its argument table on first use — a `push_data` after that
    /// rebuilds the encoder).
    encoded: bool,
    /// The root blob being assembled (CPU view).
    root_staging: Vec<u8>,
    /// The snapshot ring (GPU view).
    root_ring: Option<Memory>,
    /// Offset of the next snapshot inside the ring.
    root_cursor: u64,
    ended: bool,
}

impl CommandBuffer {
    /// Begin recording. `usage` matches the Vulkan surface; the buffer is
    /// already begun at allocation, so the call only records intent.
    pub fn begin(&mut self, _usage: CommandBufferUsage) -> moonfield_rhi_core::Result<()> {
        Ok(())
    }

    /// Begin a render pass: create the render encoder, attach the argument
    /// table for the vertex and fragment stages, and set the viewport from
    /// the render area.
    pub fn begin_rendering(&mut self, desc: &RenderPassDesc) {
        assert!(self.encoder.is_none(), "render pass already begun");

        let pass = MTL4RenderPassDescriptor::new();
        let attachment = desc
            .color_attachments
            .first()
            .expect("RenderPassDesc needs at least one color attachment");
        // SAFETY: attachment index 0 is always valid.
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(attachment.view.texture()));
        color.setLoadAction(attachment.load.to_metal());
        color.setStoreAction(attachment.store.to_metal());
        color.setClearColor(attachment.clear.to_metal());

        let encoder = self
            .cmdbuf
            .renderCommandEncoderWithDescriptor(&pass)
            .expect("renderCommandEncoderWithDescriptor failed");
        encoder.setArgumentTable_atStages(
            &self.argument_table,
            MTLRenderStages::Vertex | MTLRenderStages::Fragment,
        );
        let area = &desc.render_area;
        encoder.setViewport(MTLViewport {
            originX: area.offset.x as f64,
            originY: area.offset.y as f64,
            width: area.extent.width as f64,
            height: area.extent.height as f64,
            znear: 0.0,
            zfar: 1.0,
        });
        self.encoder = Some(encoder);
        self.encoded = false;
        self.pass_rebuild = Some(PassRebuild {
            view: attachment.view.clone(),
            store: attachment.store,
            clear: attachment.clear,
            render_area: desc.render_area,
        });
    }

    /// End the current render pass.
    pub fn end_rendering(&mut self) {
        let encoder = self
            .encoder
            .take()
            .expect("end_rendering requires a begun render pass");
        encoder.endEncoding();
        self.pass_rebuild = None;
    }

    /// Rebuild the active render encoder (a `push_data` after encoded
    /// commands): the pass reloads its previous contents (`Load`) and the
    /// tracked state replays. An encoder fixes its argument table on first
    /// use, so a root change after that needs a fresh encoder.
    fn rebuild_render_encoder(&mut self) {
        let rebuild = self
            .pass_rebuild
            .as_ref()
            .expect("render pass context exists");
        let old = self
            .encoder
            .take()
            .expect("an active render encoder exists");
        old.endEncoding();

        let pass = MTL4RenderPassDescriptor::new();
        // SAFETY: attachment index 0 is always valid.
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(rebuild.view.texture()));
        // A rebuild must preserve what has already been rendered.
        color.setLoadAction(objc2_metal::MTLLoadAction::Load);
        color.setStoreAction(rebuild.store.to_metal());
        color.setClearColor(rebuild.clear.to_metal());

        let encoder = self
            .cmdbuf
            .renderCommandEncoderWithDescriptor(&pass)
            .expect("renderCommandEncoderWithDescriptor failed");
        encoder.setArgumentTable_atStages(
            &self.argument_table,
            MTLRenderStages::Vertex | MTLRenderStages::Fragment,
        );
        let area = rebuild.render_area;
        encoder.setViewport(MTLViewport {
            originX: area.offset.x as f64,
            originY: area.offset.y as f64,
            width: area.extent.width as f64,
            height: area.extent.height as f64,
            znear: 0.0,
            zfar: 1.0,
        });
        if let Some(pipeline) = self.replay_graphics.as_ref() {
            encoder.setRenderPipelineState(pipeline.pso());
        }
        self.encoder = Some(encoder);
        self.encoded = false;
    }

    /// Rebuild the compute encoder (a `push_data` after encoded commands).
    fn rebuild_compute_encoder(&mut self) {
        let old = self
            .compute_encoder
            .take()
            .expect("an active compute encoder exists");
        old.endEncoding();
        let encoder = self
            .cmdbuf
            .computeCommandEncoder()
            .expect("computeCommandEncoder failed");
        encoder.setArgumentTable(Some(&*self.argument_table));
        if let Some(pipeline) = self.replay_compute.as_ref() {
            encoder.setComputePipelineState(pipeline.pso());
        }
        self.compute_encoder = Some(encoder);
        self.encoded = false;
    }

    /// Override the dynamic viewport. The engine's clip convention (Y-up,
    /// reverse-Z) adapts here: Metal's NDC is Y-up with z∈[0,1], so the
    /// negative-height Vulkan flip becomes a positive-height viewport and
    /// the reverse-Z range maps onto [0, 1] by inverting the depth bounds.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.replay_viewport = Some(viewport);
        self.apply_viewport(viewport);
    }

    /// Apply the viewport to the active render encoder (also used to replay
    /// after an encoder rebuild).
    fn apply_viewport(&self, viewport: Viewport) {
        let encoder = self
            .encoder
            .as_ref()
            .expect("set_viewport requires a begun render pass");
        let (origin_y, height) = if viewport.height < 0.0 {
            (viewport.y + viewport.height, -viewport.height)
        } else {
            (viewport.y, viewport.height)
        };
        encoder.setViewport(MTLViewport {
            originX: viewport.x as f64,
            originY: origin_y as f64,
            width: viewport.width as f64,
            height: height as f64,
            znear: viewport.max_depth as f64,
            zfar: viewport.min_depth as f64,
        });
    }

    /// Override the dynamic scissor.
    pub fn set_scissor(&self, scissor: Rect2d) {
        let encoder = self
            .encoder
            .as_ref()
            .expect("set_scissor requires a begun render pass");
        // SAFETY: one valid scissor rect at index 0.
        let rect = objc2_metal::MTLScissorRect {
            x: scissor.offset.x as NSUInteger,
            y: scissor.offset.y as NSUInteger,
            width: scissor.extent.width as NSUInteger,
            height: scissor.extent.height as NSUInteger,
        };
        // SAFETY: `rect` is a live stack value; count matches.
        unsafe { encoder.setScissorRects_count(std::ptr::NonNull::from(&rect), 1) };
    }

    /// Set the dynamic depth test/write/compare state. The skeleton's render
    /// passes carry no depth attachment, so this records intent only.
    pub fn set_depth_state(&self, _state: DepthState) {}

    /// Set the dynamic cull/winding state. The engine's Y-up convention
    /// pairs with reversed winding on the Vulkan side; Metal's Y-up NDC
    /// takes the front face as declared.
    pub fn set_cull_state(&self, state: CullState) {
        let encoder = self
            .encoder
            .as_ref()
            .expect("set_cull_state requires a begun render pass");
        let winding = match state.front_face {
            FrontFace::Clockwise => objc2_metal::MTLWinding::Clockwise,
            FrontFace::CounterClockwise => objc2_metal::MTLWinding::CounterClockwise,
        };
        encoder.setFrontFacingWinding(winding);
        let mode = match state.cull_mode {
            CullMode::None => objc2_metal::MTLCullMode::None,
            CullMode::Front => objc2_metal::MTLCullMode::Front,
            CullMode::Back => objc2_metal::MTLCullMode::Back,
        };
        encoder.setCullMode(mode);
    }

    /// Set the dynamic blend state (premultiplied-alpha on/off). Blend is
    /// pipeline state on Metal; blended pipelines come with the egui port.
    pub fn set_blend_state(&self, _blend: BlendMode) {}

    /// Bind the graphics pipeline for the current render pass.
    pub fn bind_graphics_pipeline(&mut self, pipeline: &GraphicsPipeline) {
        self.replay_graphics = Some(pipeline.clone());
        let encoder = self
            .encoder
            .as_ref()
            .expect("bind_graphics_pipeline requires a begun render pass");
        encoder.setRenderPipelineState(pipeline.pso());
    }

    /// Draw `vertex_count` vertices across `instance_count` instances.
    pub fn draw(
        &mut self,
        vertex_count: u32,
        instance_count: u32,
        first_vertex: u32,
        first_instance: u32,
    ) {
        let encoder = self
            .encoder
            .as_ref()
            .expect("draw requires a begun render pass");
        self.encoded = true;
        // SAFETY: a plain draw with valid counts.
        unsafe {
            encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::Triangle,
                first_vertex as usize,
                vertex_count as usize,
                instance_count as usize,
                first_instance as usize,
            );
        }
    }

    /// Begin a compute pass: create the compute encoder and attach the
    /// argument table.
    pub fn begin_compute(&mut self) {
        assert!(
            self.encoder.is_none() && self.compute_encoder.is_none(),
            "a pass is already begun"
        );
        let encoder = self
            .cmdbuf
            .computeCommandEncoder()
            .expect("computeCommandEncoder failed");
        encoder.setArgumentTable(Some(&*self.argument_table));
        self.compute_encoder = Some(encoder);
    }

    /// Bind the compute pipeline for the current compute pass.
    pub fn bind_pipeline(&mut self, pipeline: &ComputePipeline) {
        self.replay_compute = Some(pipeline.clone());
        self.threads_per_group = Some(pipeline.threads_per_group());
        let encoder = self
            .compute_encoder
            .as_ref()
            .expect("bind_pipeline requires a begun compute pass");
        encoder.setComputePipelineState(pipeline.pso());
    }

    /// Dispatch `group_x`×`group_y`×`group_z` threadgroups with the
    /// pipeline's threads-per-threadgroup.
    pub fn dispatch(&mut self, group_x: u32, group_y: u32, group_z: u32) {
        self.encoded = true;
        let encoder = self
            .compute_encoder
            .as_ref()
            .expect("dispatch requires a begun compute pass");
        let threads = self
            .threads_per_group
            .expect("dispatch requires a bound compute pipeline");
        encoder.dispatchThreadgroups_threadsPerThreadgroup(
            MTLSize {
                width: group_x as usize,
                height: group_y as usize,
                depth: group_z as usize,
            },
            threads,
        );
    }

    /// End the current compute pass.
    pub fn end_compute(&mut self) {
        let encoder = self
            .compute_encoder
            .take()
            .expect("end_compute requires a begun compute pass");
        encoder.endEncoding();
    }

    /// Bind a buffer's device address at argument-table slot `index`
    /// (shaders see it as `[[buffer(index)]]`).
    pub fn set_buffer(&mut self, index: u32, allocation: &GpuAllocation) {
        // SAFETY: the argument table has 8 buffer slots (see
        // `CommandPool::allocate`); the address belongs to a live allocation.
        unsafe {
            self.argument_table
                .setAddress_atIndex(allocation.gpu_ptr().as_raw(), index as usize)
        };
    }

    /// Push root data at `offset` into the root blob and rebind it: the
    /// bytes are snapshotted into a private ring and argument-table slot 0
    /// is rebound to the snapshot, so each subsequent draw/dispatch reads
    /// the blob as of the last push (Vulkan push-data semantics).
    ///
    /// Slang's metal codegen consumes the root blob as an
    /// `EntryPointParams` struct at `[[buffer(0)]]`.
    pub fn push_data(&mut self, offset: u32, data: &[u8]) {
        let offset = offset as usize;
        let end = offset + data.len();
        if self.root_staging.len() < end {
            self.root_staging.resize(end, 0);
        }
        self.root_staging[offset..end].copy_from_slice(data);
        self.snapshot_root();
    }

    /// Push the two-pointer bindless root (`input`, `output`).
    pub fn set_bindless_root(&mut self, input: GpuPtr, output: GpuPtr) {
        let root: [u64; 2] = [input.as_raw(), output.as_raw()];
        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&root[0].to_le_bytes());
        bytes[8..16].copy_from_slice(&root[1].to_le_bytes());
        self.push_data(0, &bytes);
    }

    /// Memory barrier between stages (encoder barriers, Metal 4).
    pub fn barrier(
        &self,
        before: Stage,
        _before_access: Access,
        after: Stage,
        _after_access: Access,
    ) {
        let before = before.to_metal();
        let after = after.to_metal();
        if let Some(encoder) = self.encoder.as_ref() {
            encoder.barrierAfterEncoderStages_beforeEncoderStages_visibilityOptions(
                before,
                after,
                objc2_metal::MTL4VisibilityOptions::Device,
            );
        }
        if let Some(encoder) = self.compute_encoder.as_ref() {
            encoder.barrierAfterEncoderStages_beforeEncoderStages_visibilityOptions(
                before,
                after,
                objc2_metal::MTL4VisibilityOptions::Device,
            );
        }
    }

    /// Snapshot the assembled root blob into the ring, bind it on a fresh
    /// argument table, and attach that table to the active encoders.
    ///
    /// Encoder bindings snapshot the argument table on first use: reusing
    /// one table cannot rebind between draws, so each push gets its own
    /// table (measured behavior; see the `push_data_snapshots` GPU test).
    fn snapshot_root(&mut self) {
        let len = self.root_staging.len() as u64;
        if self.root_ring.is_none() {
            self.root_ring = Some(Memory::new(&self.device, ROOT_RING_SIZE));
        }
        let ring = self.root_ring.as_ref().expect("ring exists");
        assert!(
            self.root_cursor + len <= ROOT_RING_SIZE,
            "root-blob ring exhausted"
        );
        ring.copy_host(self.root_cursor, &self.root_staging);

        let shared = self.device.ctx().shared();
        let table_descriptor = MTL4ArgumentTableDescriptor::new();
        table_descriptor.setMaxBufferBindCount(8);
        let table = shared
            .device()
            .newArgumentTableWithDescriptor_error(&table_descriptor)
            .expect("newArgumentTable failed");
        // SAFETY: slot 0 of the 8-slot table; the address is inside the ring.
        unsafe {
            table.setAddress_atIndex(ring.gpu_ptr().as_raw() + self.root_cursor, 0);
        }
        self.argument_table = table;
        // An encoder that already encoded commands fixed its argument table
        // on first use: rebuild it to pick up the new root snapshot.
        if self.encoded {
            if self.encoder.is_some() {
                self.rebuild_render_encoder();
            }
            if self.compute_encoder.is_some() {
                self.rebuild_compute_encoder();
            }
        } else {
            if let Some(encoder) = self.encoder.as_ref() {
                encoder.setArgumentTable_atStages(
                    &self.argument_table,
                    MTLRenderStages::Vertex | MTLRenderStages::Fragment,
                );
            }
            if let Some(encoder) = self.compute_encoder.as_ref() {
                encoder.setArgumentTable(Some(&*self.argument_table));
            }
        }
        self.root_cursor = (self.root_cursor + len).div_ceil(16) * 16;
    }

    /// End command-buffer recording. Called by `Device::submit_and_wait`;
    /// also callable directly for queue-level orchestration later.
    pub fn end(&mut self) {
        if !self.ended {
            self.cmdbuf.endCommandBuffer();
            self.ended = true;
        }
    }

    pub(crate) fn raw(&self) -> Retained<ProtocolObject<dyn MTL4CommandBuffer>> {
        self.cmdbuf.clone()
    }
}

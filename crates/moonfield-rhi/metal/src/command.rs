//! Command pool and command buffer: `MTL4CommandBuffer` recording with the
//! per-buffer `MTL4ArgumentTable`.

use moonfield_rhi_core::{AttachmentLayout, ClearValue, LoadOp, Rect2d, StoreOp};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTL4ArgumentTable, MTL4ArgumentTableDescriptor, MTL4CommandBuffer, MTL4CommandEncoder,
    MTL4RenderCommandEncoder, MTL4RenderPassDescriptor, MTLDevice, MTLPrimitiveType,
    MTLRenderStages, MTLViewport,
};

use crate::device::{Device, DeviceContext};
use crate::formats::ToMetal;
use crate::memory::GpuAllocation;
use crate::pipeline::GraphicsPipeline;
use crate::view::TextureView;

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
    /// Color attachments; the Metal skeleton uses the first one.
    pub color_attachments: &'a [RenderAttachment],
    /// Optional depth attachment. The Metal skeleton ignores it.
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
            cmdbuf,
            argument_table,
            encoder: None,
            ended: false,
        }
    }
}

/// An in-recording `MTL4CommandBuffer` with its per-buffer argument table.
pub struct CommandBuffer {
    cmdbuf: Retained<ProtocolObject<dyn MTL4CommandBuffer>>,
    argument_table: Retained<ProtocolObject<dyn MTL4ArgumentTable>>,
    encoder: Option<Retained<ProtocolObject<dyn MTL4RenderCommandEncoder>>>,
    ended: bool,
}

impl CommandBuffer {
    /// Begin a render pass: create the render encoder, attach the argument
    /// table for the vertex and fragment stages, and set the viewport from
    /// the render area.
    pub fn begin_render_pass(&mut self, desc: &RenderPassDesc) {
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
    }

    /// Set the graphics pipeline for the current render pass.
    pub fn set_pipeline(&mut self, pipeline: &GraphicsPipeline) {
        let encoder = self
            .encoder
            .as_ref()
            .expect("set_pipeline requires a begun render pass");
        encoder.setRenderPipelineState(pipeline.pso());
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

    /// Draw `vertex_count` non-indexed vertices as a triangle list.
    pub fn draw(&mut self, vertex_count: u32) {
        let encoder = self
            .encoder
            .as_ref()
            .expect("draw requires a begun render pass");
        // SAFETY: a plain draw with a valid vertex count.
        unsafe {
            encoder.drawPrimitives_vertexStart_vertexCount(
                MTLPrimitiveType::Triangle,
                0,
                vertex_count as usize,
            )
        };
    }

    /// End the current render pass.
    pub fn end_render_pass(&mut self) {
        let encoder = self
            .encoder
            .take()
            .expect("end_render_pass requires a begun render pass");
        encoder.endEncoding();
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

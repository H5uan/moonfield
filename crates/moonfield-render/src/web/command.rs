//! Minimal command recording for the wgpu backend.
//!
//! This is deliberately a much smaller surface than the native
//! `CommandBuffer`: just enough for render algorithms to draw into an
//! [`OffscreenTarget`]. No full parity with the Vulkan command API is claimed,
//! but indirect draw commands (`draw_indirect`, `draw_indexed_indirect`,
//! `multi_draw_*`) ARE part of the parity set — they are a hard requirement
//! for GPU-driven algorithms (e.g. 3D Gaussian splatting) and so mirror the
//! native backend. The `*_indirect_count` variants are not exposed here
//! because they require a non-default wgpu device feature.

use crate::indirect::IndexFormat;
use crate::web::buffer::Buffer;
use crate::web::device::Device;
use crate::web::offscreen::OffscreenTarget;
use crate::web::pipeline::GraphicsPipeline;
use std::ops::Range;

/// A wgpu command encoder wrapper.
pub struct CommandEncoder {
    inner: wgpu::CommandEncoder,
}

impl CommandEncoder {
    /// Create a command encoder.
    pub fn new(device: &Device) -> Self {
        Self {
            inner: device
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("moonfield-command-encoder"),
                }),
        }
    }

    /// Begin a render pass into `target`, clearing it to `clear` (RGBA).
    ///
    /// The returned pass borrows this encoder mutably; drop it before
    /// calling [`finish`](Self::finish).
    pub fn begin_render_pass(
        &mut self,
        target: &OffscreenTarget,
        clear: [f64; 4],
    ) -> RenderPass<'_> {
        let inner = self.inner.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("moonfield-render-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.texture_view().raw_wgpu(),
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: clear[0],
                        g: clear[1],
                        b: clear[2],
                        a: clear[3],
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        RenderPass { inner }
    }

    /// Finish recording and submit the commands to the device queue.
    pub fn finish(self, device: &Device) {
        device.queue().submit([self.inner.finish()]);
    }
}

/// A borrowing render pass handle, see
/// [`CommandEncoder::begin_render_pass`].
pub struct RenderPass<'a> {
    inner: wgpu::RenderPass<'a>,
}

impl<'a> RenderPass<'a> {
    /// Bind a graphics pipeline.
    pub fn set_pipeline(&mut self, pipeline: &'a GraphicsPipeline) {
        self.inner.set_pipeline(pipeline.raw());
    }

    /// Bind a vertex buffer to a slot.
    pub fn set_vertex_buffer(&mut self, slot: u32, buffer: &'a Buffer) {
        self.inner.set_vertex_buffer(slot, buffer.raw().slice(..));
    }

    /// Bind an index buffer with the given element format.
    pub fn set_index_buffer(&mut self, buffer: &'a Buffer, format: IndexFormat) {
        self.inner
            .set_index_buffer(buffer.raw().slice(..), format.to_wgpu());
    }

    /// Draw non-indexed primitives for the given vertex range.
    pub fn draw(&mut self, vertices: Range<u32>) {
        self.inner.draw(vertices, 0..1);
    }

    /// Draw indexed primitives for the given index range.
    pub fn draw_indexed(&mut self, indices: Range<u32>, base_vertex: i32, instances: Range<u32>) {
        self.inner.draw_indexed(indices, base_vertex, instances);
    }

    /// Issue a single non-indexed indirect draw from `indirect` at `offset`.
    pub fn draw_indirect(&mut self, indirect: &'a Buffer, offset: u64) {
        self.inner.draw_indirect(indirect.raw(), offset);
    }

    /// Issue a single indexed indirect draw from `indirect` at `offset`.
    pub fn draw_indexed_indirect(&mut self, indirect: &'a Buffer, offset: u64) {
        self.inner.draw_indexed_indirect(indirect.raw(), offset);
    }

    /// Issue `count` non-indexed indirect draws from `indirect` at `offset`.
    pub fn multi_draw_indirect(&mut self, indirect: &'a Buffer, offset: u64, count: u32) {
        self.inner
            .multi_draw_indirect(indirect.raw(), offset, count);
    }

    /// Issue `count` indexed indirect draws from `indirect` at `offset`.
    pub fn multi_draw_indexed_indirect(&mut self, indirect: &'a Buffer, offset: u64, count: u32) {
        self.inner
            .multi_draw_indexed_indirect(indirect.raw(), offset, count);
    }
}

//! Minimal command recording for the wgpu backend.
//!
//! This is deliberately a much smaller surface than the native
//! `CommandBuffer`: just enough for render algorithms to draw into an
//! [`OffscreenTarget`]. No parity with the Vulkan command API is claimed.

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

    /// Draw non-indexed primitives for the given vertex range.
    pub fn draw(&mut self, vertices: Range<u32>) {
        self.inner.draw(vertices, 0..1);
    }
}

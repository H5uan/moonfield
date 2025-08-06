use std::rc::Weak;

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLIndexType,
    MTLLoadAction, MTLPrimitiveType, MTLRenderCommandEncoder,
    MTLRenderPassDescriptor,
};
use objc2_quartz_core::CAMetalDrawable;

use crate::{
    error::{GraphicsError, MetalError},
    frame_buffer::FrameBuffer,
    geometry_buffer::{GeometryBufferAsAny, GeometryBufferWarpper},
    metal_backend::{
        MetalGraphicsBackend, geometry_buffer::MetalGeometryBuffer,
    },
};

pub struct MetalFrameBuffer {
    pub backend: Weak<MetalGraphicsBackend>,
    pub drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    pub render_pass_descriptor: Retained<MTLRenderPassDescriptor>,
    /// command_buffer: represents a collection of render commands to be executed as a unit
    /// each command buffer is associated with a queue
    pub command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    /// render_encoder: An object that is used to tell Metal what drawing we actually want to do
    /// It will translate high-level commands (shader params, draw call, etc.) into low-level
    /// instructions that are then written into its corresponding command buffer
    pub render_encoder:
        Option<Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>>,
    // Framebuffer size
    pub width: u32,
    pub height: u32,
}

impl MetalFrameBuffer {
    fn get_or_create_render_encoder(
        &mut self,
    ) -> Result<&ProtocolObject<dyn MTLRenderCommandEncoder>, GraphicsError>
    {
        if self.render_encoder.is_none() {
            let encoder = self
                .command_buffer
                .renderCommandEncoderWithDescriptor(
                    &self.render_pass_descriptor,
                )
                .ok_or_else(|| {
                    GraphicsError::MetalError(MetalError::RenderPassError(
                        "Failed to create render command encoder".to_string(),
                    ))
                })?;
            self.render_encoder = Some(encoder);
        }

        Ok(self.render_encoder.as_ref().unwrap())
    }
}

impl FrameBuffer for MetalFrameBuffer {
    fn clear(&self, color: [f32; 4]) -> Result<(), GraphicsError> {
        let color_attachment = unsafe {
            self.render_pass_descriptor
                .colorAttachments()
                .objectAtIndexedSubscript(0)
        };

        // Set the clear color
        color_attachment.setClearColor(MTLClearColor {
            red: color[0] as f64,
            green: color[1] as f64,
            blue: color[2] as f64,
            alpha: color[3] as f64,
        });

        // Set load action to clear the buffer with the specified color
        color_attachment.setLoadAction(MTLLoadAction::Clear);

        Ok(())
    }

    fn draw(
        &mut self, geometry_buffer: &GeometryBufferWarpper,
    ) -> Result<(), GraphicsError> {
        let backend =
            self.backend.upgrade().ok_or(GraphicsError::BackendUnavailable)?;

        let metal_geometry = geometry_buffer
            .0
            .as_ref()
            .as_any()
            .downcast_ref::<MetalGeometryBuffer>()
            .ok_or(GraphicsError::BackendUnavailable)?;

        let encoder = self.get_or_create_render_encoder()?;

        encoder.setRenderPipelineState(&backend.pipeline_state);

        for (index, vertex_buffer_option) in
            metal_geometry.vertex_buffers().iter().enumerate()
        {
            if let Some(vertex_buffer) = vertex_buffer_option {
                unsafe {
                    encoder.setVertexBuffer_offset_atIndex(
                        Some(&vertex_buffer.buffer),
                        0,
                        index,
                    )
                };
            }
        }

        if let Some(index_buffer) = &metal_geometry.index_buffer() {
            unsafe {
                encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset(
        MTLPrimitiveType::Triangle,
        metal_geometry.index_count() as usize,
        MTLIndexType::UInt32,
        &index_buffer.buffer,
        0
    )
            };
        } else {
            unsafe {
                encoder.drawPrimitives_vertexStart_vertexCount(
                    MTLPrimitiveType::Triangle,
                    0,
                    metal_geometry.vertex_count() as usize,
                )
            };
        }

        Ok(())
    }
}

impl Drop for MetalFrameBuffer {
    fn drop(&mut self) {
        match self.render_encoder.take() {
            Some(encoder) => {
                // End the existing render encoder
                encoder.endEncoding();
            }
            None => {
                // No active encoder, create one to execute any pending operations (like clear)
                let encoder =
                    self.command_buffer.renderCommandEncoderWithDescriptor(
                        &self.render_pass_descriptor,
                    );
                if let Some(encoder) = encoder {
                    encoder.endEncoding();
                }
            }
        }

        self.command_buffer
            .presentDrawable(ProtocolObject::from_ref(&*self.drawable));
        self.command_buffer.commit();
    }
}

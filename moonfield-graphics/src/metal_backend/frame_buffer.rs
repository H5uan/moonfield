use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLLoadAction,
    MTLRenderCommandEncoder, MTLRenderPassDescriptor,
};
use objc2_quartz_core::CAMetalDrawable;

use crate::{
    error::{GraphicsError, MetalError},
    frame_buffer::FrameBuffer,
};

pub struct MetalFrameBuffer {
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

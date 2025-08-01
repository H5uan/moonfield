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
    pub command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    pub render_encoder:
        Option<Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>>,
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
        // If no render encoder was explicitly created, create one to execute any pending operations (like clear)
        if self.render_encoder.is_none() {
            let encoder =
                self.command_buffer.renderCommandEncoderWithDescriptor(
                    &self.render_pass_descriptor,
                );
            if let Some(encoder) = encoder {
                encoder.endEncoding();
            }
        }

        self.command_buffer
            .presentDrawable(ProtocolObject::from_ref(&*self.drawable));
        self.command_buffer.commit();
    }
}

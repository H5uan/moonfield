use crate::{types::*, CommandBuffer, RenderPassDescriptor, SwapchainImage};

// Import tracing for logging
use tracing;

pub struct MetalCommandBuffer {
    pub command_buffer: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLCommandBuffer>>,
    pub current_encoder: Option<objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLRenderCommandEncoder>>>,
}

impl CommandBuffer for MetalCommandBuffer {
    fn begin(&self) -> Result<(), RhiError> {
        Ok(())
    }

    fn end(&self) -> Result<(), RhiError> {
        Ok(())
    }

    fn begin_render_pass(&self, desc: &RenderPassDescriptor, image: &SwapchainImage) {
        unsafe {
            let render_pass_desc = objc2_metal::MTLRenderPassDescriptor::new();
            
            let color_attachment = render_pass_desc
                .colorAttachments()
                .objectAtIndexedSubscript(0);
            
            let clear = &desc.color_attachments[0].clear_value;
            color_attachment.setClearColor(objc2_metal::MTLClearColor {
                red: clear[0] as f64,
                green: clear[1] as f64,
                blue: clear[2] as f64,
                alpha: clear[3] as f64,
            });

            let load_action = match desc.color_attachments[0].load_op {
                LoadOp::Load => objc2_metal::MTLLoadAction::Load,
                LoadOp::Clear => objc2_metal::MTLLoadAction::Clear,
                LoadOp::DontCare => objc2_metal::MTLLoadAction::DontCare,
            };
            color_attachment.setLoadAction(load_action);

            let store_action = match desc.color_attachments[0].store_op {
                StoreOp::Store => objc2_metal::MTLStoreAction::Store,
                StoreOp::DontCare => objc2_metal::MTLStoreAction::DontCare,
            };
            color_attachment.setStoreAction(store_action);
        }
    }

    fn end_render_pass(&self) {
        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.endEncoding();
            }
        }
    }

    fn bind_pipeline(&self, pipeline: &dyn crate::Pipeline) {
        let pipeline_any = pipeline as &dyn std::any::Any;
        let metal_pipeline = pipeline_any.downcast_ref::<super::pipeline::MetalPipeline>().unwrap();

        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.setRenderPipelineState(&metal_pipeline.pipeline_state);
            }
        }
    }

    fn bind_vertex_buffer(&self, buffer: &dyn crate::Buffer) {
        let buffer_any = buffer as &dyn std::any::Any;
        let metal_buffer = buffer_any.downcast_ref::<super::buffer::MetalBuffer>().unwrap();

        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.setVertexBuffer_offset_atIndex(Some(&metal_buffer.buffer), 0, 0);
            }
        }
    }

    fn set_viewport(&self, width: f32, height: f32) {}

    fn set_scissor(&self, width: u32, height: u32) {}
    
    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        unsafe {
            if let Some(encoder) = &self.current_encoder {
                encoder.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                    objc2_metal::MTLPrimitiveType::Triangle,
                    first_vertex as usize,
                    vertex_count as usize,
                    instance_count as usize,
                    first_instance as usize,
                );
            }
        }
    }
}
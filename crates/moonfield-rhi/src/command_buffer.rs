use std::any::Any;


use crate::types::{RenderPassDescriptor, RhiError, SwapchainImage};
use crate::{Buffer, Pipeline};

/// Trait for command buffer functionality
pub trait CommandBuffer: Any {
    /// Begins recording commands to this command buffer
    fn begin(&self) -> Result<(), RhiError>;
    
    /// Ends recording commands to this command buffer
    fn end(&self) -> Result<(), RhiError>;
    
    /// Begins a render pass
    fn begin_render_pass(&self, desc: &RenderPassDescriptor, image: &SwapchainImage);
    
    /// Ends the current render pass
    fn end_render_pass(&self);
    
    /// Sets the viewport for rendering
    fn set_viewport(&self, width: f32, height: f32);
    
    /// Sets the scissor rectangle for rendering
    fn set_scissor(&self, width: u32, height: u32);
    
    /// Binds a graphics pipeline to this command buffer
    fn bind_pipeline(&self, pipeline: &dyn Pipeline);
    
    /// Binds a vertex buffer to this command buffer
    fn bind_vertex_buffer(&self, buffer: &dyn Buffer);
    
    /// Draws primitives
    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32);
}
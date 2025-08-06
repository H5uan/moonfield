use std::rc::Rc;

use moonfield_core::math::{color, color::Color};
use moonfield_graphics::{
    backend::{GraphicsBackend, SharedGraphicsBackend},
    error::GraphicsError,
    geometry_buffer::GeometryBufferWarpper,
};

use crate::engine::error::EngineError;

pub struct Renderer {
    frame_size: (u32, u32),
    pub backend: SharedGraphicsBackend,

    // Clear color
    clear_color: Color,
    
    // Geometry buffers to render this frame
    geometry_buffers: Vec<GeometryBufferWarpper>,
}

impl Renderer {
    pub fn new(
        backend: Rc<dyn GraphicsBackend>, frame_size: (u32, u32),
    ) -> Result<Self, EngineError> {
        Ok(Self { 
            frame_size, 
            backend, 
            clear_color: color::BLACK,
            geometry_buffers: Vec::new(),
        })
    }

    pub(crate) fn render_frame(&mut self) -> Result<(), GraphicsError> {
        let mut back_buffer = self.backend.back_buffer()?;

        // Use the renderer's clear color
        back_buffer.clear([
            self.clear_color.x,
            self.clear_color.y,
            self.clear_color.z,
            self.clear_color.w,
        ])?;

        // Draw all geometry buffers
        for geometry_buffer in &self.geometry_buffers {
            back_buffer.draw(geometry_buffer)?;
        }

        drop(back_buffer);

        self.backend.swap_buffers()?;

        Ok(())
    }
    pub fn graphics_backend(&self) -> SharedGraphicsBackend {
        self.backend.clone()
    }

    pub(crate) fn set_frame_size(
        &mut self, new_size: (u32, u32),
    ) -> Result<(), GraphicsError> {
        self.frame_size.0 = new_size.0;
        self.frame_size.1 = new_size.1;

        self.graphics_backend().set_frame_size(new_size);

        Ok(())
    }

    /// Set the clear color for the renderer
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Get the current clear color
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Get the current frame size
    pub fn frame_size(&self) -> (u32, u32) {
        self.frame_size
    }

    /// Add a geometry buffer to be rendered this frame
    pub fn draw_geometry(&mut self, geometry_buffer: GeometryBufferWarpper) {
        self.geometry_buffers.push(geometry_buffer);
    }

    /// Clear all geometry buffers (usually called at the end of each frame)
    pub fn clear_geometry(&mut self) {
        self.geometry_buffers.clear();
    }
}

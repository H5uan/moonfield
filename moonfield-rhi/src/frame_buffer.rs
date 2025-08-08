use crate::{error::GraphicsError, geometry_buffer::GeometryBufferWarpper};

pub trait FrameBuffer {
    fn clear(&self, color: [f32; 4]) -> Result<(), GraphicsError>;
    fn draw(
        &mut self, geometry: &GeometryBufferWarpper,
    ) -> Result<(), GraphicsError>;
}

pub type SharedFrameBuffer = Box<dyn FrameBuffer>;

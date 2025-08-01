use crate::error::GraphicsError;

pub trait FrameBuffer {
    fn clear(&self, color: [f32; 4]) -> Result<(), GraphicsError>;
}

pub type SharedFrameBuffer = Box<dyn FrameBuffer>;

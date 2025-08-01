use std::rc::Rc;

use crate::{error::GraphicsError, frame_buffer::SharedFrameBuffer};

pub type SharedGraphicsBackend = Rc<dyn GraphicsBackend>;

#[derive(Debug)]
pub struct BackendCapabilities {
    pub max_buffer_length: usize,
}

// all graphics backend has some equal operation
// like buffer and texture management
pub trait GraphicsBackend {
    fn back_buffer(&self) -> Result<SharedFrameBuffer, GraphicsError>;

    fn swap_buffers(&self) -> Result<(), GraphicsError>;

    fn set_frame_size(&self, new_size: (u32, u32));

    fn capabilities(&self) -> BackendCapabilities;
}

use std::rc::Rc;

use crate::{
    error::GraphicsError,
    frame_buffer::SharedFrameBuffer,
    geometry_buffer::{
        self, GeometryBuffer, GeometryBufferDescriptor, GeometryBufferWarpper,
    },
};

pub type SharedGraphicsBackend = Rc<dyn Device>;

#[derive(Debug)]
pub struct BackendCapabilities {
    pub max_buffer_length: usize,
}

// all graphics backend has some equal operation
// like buffer and texture management
pub trait Device {
    fn back_buffer(&self) -> Result<SharedFrameBuffer, GraphicsError>;

    fn swap_buffers(&self) -> Result<(), GraphicsError>;

    fn set_frame_size(&self, new_size: (u32, u32));

    fn create_geometry_buffer(
        &self, desc: GeometryBufferDescriptor,
    ) -> Result<GeometryBufferWarpper, GraphicsError>;

    fn capabilities(&self) -> BackendCapabilities;
}

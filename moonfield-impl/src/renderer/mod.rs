use std::rc::Rc;

use moonfield_graphics::{
    backend::{GraphicsBackend, SharedGraphicsBackend},
    error::GraphicsError,
};

use crate::engine::error::EngineError;

pub struct Renderer {
    frame_size: (u32, u32),
    pub backend: SharedGraphicsBackend,
}

impl Renderer {
    pub fn new(
        backend: Rc<dyn GraphicsBackend>, frame_size: (u32, u32),
    ) -> Result<Self, EngineError> {
        Ok(Self { frame_size, backend })
    }

    pub(crate) fn render_frame(&mut self) -> Result<(), GraphicsError> {
        let back_buffer = self.backend.back_buffer()?;

        back_buffer.clear([1.0, 0.0, 0.0, 1.0])?;

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
}

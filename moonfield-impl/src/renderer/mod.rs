use std::rc::Rc;

use moonfield_graphics::backend::{GraphicsBackend, SharedGraphicsBackend};

use crate::engine::error::EngineError;

pub struct Renderer {
    frame_size: (u32, u32),
    pub backend: SharedGraphicsBackend,
}

impl Renderer {
    pub fn new(
        backend: Rc<dyn GraphicsBackend>,
        frame_size: (u32, u32),
    ) -> Result<Self, EngineError> {
        Ok(Self {
            frame_size,
            backend,
        })
    }
}

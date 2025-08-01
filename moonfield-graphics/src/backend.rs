use std::rc::Rc;

pub type SharedGraphicsBackend = Rc<dyn GraphicsBackend>;

// all graphics backend has some equal operation
// like buffer and texture management
pub trait GraphicsBackend {}

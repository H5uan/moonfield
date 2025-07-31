use std::rc::Rc;

pub type SharedGraphicsBackend = Rc<dyn GraphicsBackend>;

pub trait GraphicsBackend {}

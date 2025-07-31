use std::rc::Rc;

use moonfield_graphics::{backend::SharedGraphicsBackend, error::GraphicsError};
use winit::window::{Window, WindowAttributes};

use crate::renderer::Renderer;

pub struct InitilizedGraphicsContext {
    pub window: Window,
    pub renderer: Renderer,
    params: GraphicsContextParams,
}

pub type GraphicsBackendConstructorResult = Result<(Window, SharedGraphicsBackend), GraphicsError>;

pub type GraphicsBackendConstructorCallback = dyn Fn();

pub struct GraphicsBackendConstructor(Rc<GraphicsBackendConstructorCallback>);

pub struct GraphicsContextParams {
    pub window_attributes: WindowAttributes,

    pub vsync: bool,

    pub msaa_sample_count: Option<u8>,

    pub graphics_backend_constructor: GraphicsBackendConstructor,

    // To assign meaningful names for GPU objects
    pub named_objects: bool,
}

#[allow(clippy::large_enum_variant)]
pub enum GraphicsContext {
    Initilized(InitilizedGraphicsContext),
    UnInitialized(GraphicsContextParams),
}

pub struct Engine {
    pub graphics: GraphicsContext,

    // Amount of time (in seconds) that passed from creation of the engine
    elapsed_time: f32,
}

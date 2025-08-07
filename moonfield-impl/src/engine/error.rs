use std::fmt::{Display, Formatter};

use moonfield_graphics::error::GraphicsError;
use tracing::error;

use crate::renderer::error::RendererError;

#[derive(Debug)]
pub enum EngineError {
    /// Graphics system error
    Graphics(GraphicsError),

    /// Renderer system error
    Renderer(RendererError),

    /// Internal error
    Custom(String),
}

impl Display for EngineError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Graphics(v) => Display::fmt(v, f),
            EngineError::Renderer(v) => Display::fmt(v, f),

            EngineError::Custom(v) => {
                write!(f, "Custom error: {v}")
            }
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EngineError::Graphics(err) => Some(err),
            EngineError::Renderer(err) => Some(err),
            EngineError::Custom(_) => None,
        }
    }
}

impl From<GraphicsError> for EngineError {
    fn from(graphics_error: GraphicsError) -> Self {
        Self::Graphics(graphics_error)
    }
}

impl From<RendererError> for EngineError {
    fn from(renderer_error: RendererError) -> Self {
        Self::Renderer(renderer_error)
    }
}

impl EngineError {
    pub fn custom(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Engine custom error: {}", msg);
        EngineError::Custom(msg)
    }

    pub fn log_and_return(self) -> Self {
        error!("Engine error: {}", self);
        self
    }
}

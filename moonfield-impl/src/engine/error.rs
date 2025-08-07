use moonfield_graphics::error::GraphicsError;
use thiserror::Error;
use tracing::error;

use crate::renderer::error::RendererError;

#[derive(Debug, Error)]
pub enum EngineError {
    /// Graphics system error
    #[error("Graphics error: {0}")]
    Graphics(#[from] GraphicsError),

    /// Renderer system error
    #[error("Renderer error: {0}")]
    Renderer(#[from] RendererError),

    /// Internal error
    #[error("Custom error: {0}")]
    Custom(String),
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

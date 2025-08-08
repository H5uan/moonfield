use moonfield_rhi::error::GraphicsError;
use thiserror::Error;

use crate::renderer::error::RendererError;

#[derive(Debug, Error)]
pub enum EngineError {
    /// Graphics system error
    #[error(transparent)]
    Graphics(#[from] GraphicsError),

    /// Renderer system error
    #[error(transparent)]
    Renderer(#[from] RendererError),

    /// Internal error
    #[error("{0}")]
    Custom(String),
}

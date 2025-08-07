use thiserror::Error;
use tracing::error;

#[derive(Debug, Error)]
pub enum RendererError {
    #[error("Invalid frame size: {0}")]
    InvalidFrameSize(String),

    #[error("Back buffer unavailable")]
    BackBufferUnavailable,

    #[error("Geometry buffer overflow")]
    GeometryBufferOverflow,

    #[error("Render pass failed: {0}")]
    RenderPassFailed(String),

    #[error("Clear operation failed: {0}")]
    ClearFailed(String),
}

// Helper methods
impl RendererError {
    /// Create an invalid frame size error and log it
    pub fn invalid_frame_size(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Renderer invalid frame size error: {}", msg);
        RendererError::InvalidFrameSize(msg)
    }

    /// Create a render pass failed error and log it
    pub fn render_pass_failed(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Renderer render pass failed: {}", msg);
        RendererError::RenderPassFailed(msg)
    }

    /// Create a clear failed error and log it
    pub fn clear_failed(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Renderer clear failed: {}", msg);
        RendererError::ClearFailed(msg)
    }

    /// Log the error and return self
    pub fn log_and_return(self) -> Self {
        error!("Renderer error: {}", self);
        self
    }
}

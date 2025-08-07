use std::fmt::{Display, Formatter};
use tracing::error;

#[derive(Debug)]
pub enum RendererError {
    InvalidFrameSize(String),
    BackBufferUnavailable,
    GeometryBufferOverflow,
    RenderPassFailed(String),
    ClearFailed(String),
}

impl Display for RendererError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RendererError::InvalidFrameSize(msg) => {
                write!(f, "Invalid frame size: {}", msg)
            }
            RendererError::BackBufferUnavailable => {
                write!(f, "Back buffer unavailable")
            }
            RendererError::GeometryBufferOverflow => {
                write!(f, "Geometry buffer overflow")
            }
            RendererError::RenderPassFailed(msg) => {
                write!(f, "Render pass failed: {}", msg)
            }
            RendererError::ClearFailed(msg) => {
                write!(f, "Clear operation failed: {}", msg)
            }
        }
    }
}

impl std::error::Error for RendererError {}

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

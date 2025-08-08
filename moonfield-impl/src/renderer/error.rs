use thiserror::Error;

#[derive(Debug, Error)]
pub enum RendererError {
    #[error("Invalid frame size: {0}")]
    InvalidFrameSize(String),

    #[error("Back buffer unavailable")]
    BackBufferUnavailable,

    #[error("Geometry buffer overflow")]
    GeometryBufferOverflow,

    #[error("Render operation failed: {0}")]
    RenderOperationFailed(String),
}

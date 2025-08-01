use std::fmt::{Display, Formatter};

use moonfield_graphics::error::GraphicsError;
use tracing::error;

#[derive(Debug)]
pub enum EngineError {
    /// Graphics system Error
    Graphics(GraphicsError),
    /// Internal error
    Custom(String),
}

impl Display for EngineError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Graphics(v) => Display::fmt(v, f),
            EngineError::Custom(v) => {
                write!(f, "Custom error: {v}")
            }
        }
    }
}

impl From<GraphicsError> for EngineError {
    fn from(graphics_error: GraphicsError) -> Self {
        error!("Graphics error converted to engine error: {}", graphics_error);
        Self::Graphics(graphics_error)
    }
}

impl EngineError {
    /// Create a custom error and log it
    pub fn custom(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Engine custom error: {}", msg);
        EngineError::Custom(msg)
    }

    /// Log the error and return self
    pub fn log_error(self) -> Self {
        error!("Engine error: {}", self);
        self
    }
}

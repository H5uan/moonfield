use std::fmt::{Display, Formatter};

use moonfield_graphics::error::GraphicsError;

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
        Self::Graphics(graphics_error)
    }
}

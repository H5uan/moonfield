use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum GraphicsError {
    GraphicsBackendUnavailable,
    VulkanLoadError(String),
    VulkanInstanceCreationError(String),
    Custom(String),
}

impl Display for GraphicsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphicsError::GraphicsBackendUnavailable => {
                write!(f, "Graphics backend unavailable/disconnected")
            }
            GraphicsError::VulkanLoadError(msg) => {
                write!(f, "Failed to load Vulkan: {}", msg)
            }
            GraphicsError::VulkanInstanceCreationError(msg) => {
                write!(f, "Failed to create Vulkan instance: {}", msg)
            }
            GraphicsError::Custom(v) => {
                write!(f, "Custom error: {v}")
            }
        }
    }
}

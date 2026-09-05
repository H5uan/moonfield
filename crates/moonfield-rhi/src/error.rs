//! Render error types.

use std::fmt;

/// Render-specific result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in the rendering interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A requested device capability is not supported.
    Unsupported(String),
    /// A Vulkan API call returned an error.
    Backend(String),
    /// A resource handle was invalid or already destroyed.
    InvalidHandle,
    /// Shader compilation failed.
    ShaderCompilation(String),
    /// No suitable graphics adapter was found.
    AdapterRequest(String),
    /// The logical device request failed.
    DeviceRequest(String),
    /// The swapchain is out of date for its surface and must be recreated.
    SurfaceOutOfDate,
    /// Validation failed.
    Validation(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(msg) => write!(f, "unsupported operation: {}", msg),
            Error::Backend(msg) => write!(f, "backend error: {}", msg),
            Error::InvalidHandle => write!(f, "invalid handle"),
            Error::ShaderCompilation(msg) => write!(f, "shader compilation failed: {}", msg),
            Error::AdapterRequest(msg) => write!(f, "no suitable graphics adapter found: {}", msg),
            Error::DeviceRequest(msg) => write!(f, "device request failed: {}", msg),
            Error::SurfaceOutOfDate => write!(f, "swapchain is out of date"),
            Error::Validation(msg) => write!(f, "validation failed: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// Convert an ash result code. Crate-internal: keeps `ash` types out of
    /// the public API.
    pub(crate) fn from_vk(result: ash::vk::Result) -> Self {
        Error::Backend(format!("{:?}", result))
    }
}

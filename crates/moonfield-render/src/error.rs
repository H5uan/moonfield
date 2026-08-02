//! Render error types.

use std::fmt;

/// Render-specific result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in the rendering interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A requested device capability is not supported.
    Unsupported,
    /// A native API call returned an error.
    Backend(String),
    /// A resource handle was invalid or already destroyed.
    InvalidHandle,
    /// Shader compilation failed.
    ShaderCompilation(String),
    /// No suitable graphics adapter was found.
    AdapterRequest(String),
    /// The logical device request failed.
    DeviceRequest(String),
    /// Validation failed.
    Validation(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => write!(f, "unsupported operation"),
            Error::Backend(msg) => write!(f, "backend error: {}", msg),
            Error::InvalidHandle => write!(f, "invalid handle"),
            Error::ShaderCompilation(msg) => write!(f, "shader compilation failed: {}", msg),
            Error::AdapterRequest(msg) => write!(f, "no suitable graphics adapter found: {}", msg),
            Error::DeviceRequest(msg) => write!(f, "device request failed: {}", msg),
            Error::Validation(msg) => write!(f, "validation failed: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(feature = "native")]
impl From<ash::vk::Result> for Error {
    fn from(result: ash::vk::Result) -> Self {
        Error::Backend(format!("{:?}", result))
    }
}

#[cfg(feature = "native")]
impl From<ash::LoadingError> for Error {
    fn from(err: ash::LoadingError) -> Self {
        Error::Backend(format!("failed to load Vulkan: {}", err))
    }
}

#[cfg(feature = "web")]
impl From<wgpu::RequestAdapterError> for Error {
    fn from(err: wgpu::RequestAdapterError) -> Self {
        Error::AdapterRequest(err.to_string())
    }
}

#[cfg(feature = "web")]
impl From<wgpu::RequestDeviceError> for Error {
    fn from(err: wgpu::RequestDeviceError) -> Self {
        Error::DeviceRequest(err.to_string())
    }
}

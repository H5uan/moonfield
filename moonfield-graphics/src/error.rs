use std::fmt::{Display, Formatter};

use tracing::error;

#[derive(Debug)]
pub enum GraphicsError {
    BackendUnavailable,
    VulkanError(VulkanError),
    MetalError(MetalError),
    WindowCreationError(String),
    Custom(String),
}

#[derive(Debug)]
pub enum VulkanError {
    LoadError(String),
    InstanceCreationError(String),
    DeviceError(String),
    SwapchainError(String),
    CommandError(String),
}

#[derive(Debug)]
pub enum MetalError {
    DeviceCreationError(String),
    CommandQueueError(String),
    RenderPassError(String),
}

impl Display for GraphicsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphicsError::BackendUnavailable => {
                write!(f, "Graphics backend unavailable/disconnected")
            }
            GraphicsError::VulkanError(err) => {
                write!(f, "Vulkan error: {}", err)
            }
            GraphicsError::MetalError(err) => {
                write!(f, "Metal error: {}", err)
            }
            GraphicsError::WindowCreationError(msg) => {
                write!(f, "Window creation error: {}", msg)
            }
            GraphicsError::Custom(v) => {
                write!(f, "Custom error: {v}")
            }
        }
    }
}

impl Display for VulkanError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            VulkanError::LoadError(msg) => {
                write!(f, "Failed to load Vulkan: {}", msg)
            }
            VulkanError::InstanceCreationError(msg) => {
                write!(f, "Failed to create Vulkan instance: {}", msg)
            }
            VulkanError::DeviceError(msg) => {
                write!(f, "Vulkan device error: {}", msg)
            }
            VulkanError::SwapchainError(msg) => {
                write!(f, "Vulkan swapchain error: {}", msg)
            }
            VulkanError::CommandError(msg) => {
                write!(f, "Vulkan command error: {}", msg)
            }
        }
    }
}

impl Display for MetalError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            MetalError::DeviceCreationError(msg) => {
                write!(f, "Failed to create Metal device: {}", msg)
            }
            MetalError::CommandQueueError(msg) => {
                write!(f, "Metal command queue error: {}", msg)
            }
            MetalError::RenderPassError(msg) => {
                write!(f, "Metal render pass error: {}", msg)
            }
        }
    }
}

impl From<VulkanError> for GraphicsError {
    fn from(err: VulkanError) -> Self {
        error!("Vulkan error occurred: {}", err);
        GraphicsError::VulkanError(err)
    }
}

impl From<MetalError> for GraphicsError {
    fn from(err: MetalError) -> Self {
        error!("Metal error occurred: {}", err);
        GraphicsError::MetalError(err)
    }
}

impl GraphicsError {
    /// Create a custom error and log it
    pub fn custom(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Graphics custom error: {}", msg);
        GraphicsError::Custom(msg)
    }

    /// Create a window creation error and log it
    pub fn window_creation_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Window creation error: {}", msg);
        GraphicsError::WindowCreationError(msg)
    }

    /// Log the error and return self
    pub fn log_error(self) -> Self {
        error!("Graphics error: {}", self);
        self
    }
}

impl VulkanError {
    /// Create a load error and log it
    pub fn load_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Vulkan load error: {}", msg);
        VulkanError::LoadError(msg)
    }

    /// Create an instance creation error and log it
    pub fn instance_creation_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Vulkan instance creation error: {}", msg);
        VulkanError::InstanceCreationError(msg)
    }
}

impl MetalError {
    /// Create a device creation error and log it
    pub fn device_creation_error(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        error!("Metal device creation error: {}", msg);
        MetalError::DeviceCreationError(msg)
    }
}

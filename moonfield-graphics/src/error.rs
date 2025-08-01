use std::fmt::{Display, Formatter};

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
        GraphicsError::VulkanError(err)
    }
}

impl From<MetalError> for GraphicsError {
    fn from(err: MetalError) -> Self {
        GraphicsError::MetalError(err)
    }
}

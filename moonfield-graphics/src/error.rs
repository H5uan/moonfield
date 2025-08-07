use thiserror::Error;
use tracing::error;

#[derive(Debug, Error)]
pub enum GraphicsError {
    #[error("Graphics backend unavailable/disconnected")]
    BackendUnavailable,

    #[error("Vulkan error: {0}")]
    VulkanError(#[from] VulkanError),

    #[error("Metal error: {0}")]
    MetalError(#[from] MetalError),

    #[error("Window creation error: {0}")]
    WindowCreationError(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Buffer Overflow")]
    BufferOverflow,

    #[error("Custom error: {0}")]
    Custom(String),
}

#[derive(Debug, Error)]
pub enum VulkanError {
    #[error("Failed to load Vulkan: {0}")]
    LoadError(String),

    #[error("Failed to create Vulkan instance: {0}")]
    InstanceCreationError(String),

    #[error("Vulkan device error: {0}")]
    DeviceError(String),

    #[error("Vulkan swapchain error: {0}")]
    SwapchainError(String),

    #[error("Vulkan command error: {0}")]
    CommandError(String),
}

#[derive(Debug, Error)]
pub enum MetalError {
    #[error("Failed to create Metal device: {0}")]
    DeviceCreationError(String),

    #[error("Metal command queue error: {0}")]
    CommandQueueError(String),

    #[error("Metal render pass error: {0}")]
    RenderPassError(String),

    #[error("Metal buffer creation error: {0}")]
    BufferCreationError(String),

    #[error("Metal buffer mapping error: {0}")]
    BufferMappingError(String),

    #[error("Metal buffer write error: {0}")]
    BufferWriteError(String),

    #[error("Metal buffer read error: {0}")]
    BufferReadError(String),

    #[error("Failed to create shader library: {0}")]
    ShaderCompilationError(String),

    #[error("Failed to create pipeline: {0}")]
    PipelineCreationError(String),
}

// Helper methods
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
    pub fn log_and_return(self) -> Self {
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

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GraphicsError {
    #[error("Graphics backend unavailable/disconnected")]
    BackendUnavailable,

    #[error("Failed to initialize: {0}")]
    InitializationFailed(String),

    #[error("Device error: {0}")]
    DeviceError(String),

    #[error("Command error: {0}")]
    CommandError(String),

    #[error("Resource creation failed: {0}")]
    ResourceCreationFailed(String),

    #[error("Shader error: {0}")]
    ShaderError(String),

    #[error("Pipeline error: {0}")]
    PipelineError(String),

    #[error("Swapchain error: {0}")]
    SwapchainError(String),

    #[error("Buffer error: {0}")]
    BufferError(String),

    #[error("Window creation error: {0}")]
    WindowCreationError(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Buffer overflow")]
    BufferOverflow,

    #[error("{0}")]
    Custom(String),
}

// Helper methods for creating errors with context
impl GraphicsError {
    pub fn initialization_failed(backend: &str, reason: &str) -> Self {
        Self::InitializationFailed(format!("{}: {}", backend, reason))
    }

    pub fn device_error(operation: &str, reason: &str) -> Self {
        Self::DeviceError(format!("{}: {}", operation, reason))
    }

    pub fn command_error(operation: &str, reason: &str) -> Self {
        Self::CommandError(format!("{}: {}", operation, reason))
    }

    pub fn resource_creation_failed(resource: &str, reason: &str) -> Self {
        Self::ResourceCreationFailed(format!("{}: {}", resource, reason))
    }

    pub fn shader_error(stage: &str, reason: &str) -> Self {
        Self::ShaderError(format!("{}: {}", stage, reason))
    }

    pub fn pipeline_error(operation: &str, reason: &str) -> Self {
        Self::PipelineError(format!("{}: {}", operation, reason))
    }

    pub fn swapchain_error(operation: &str, reason: &str) -> Self {
        Self::SwapchainError(format!("{}: {}", operation, reason))
    }

    pub fn buffer_error(operation: &str, reason: &str) -> Self {
        Self::BufferError(format!("{}: {}", operation, reason))
    }
}

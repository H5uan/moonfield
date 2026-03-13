use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum RhiError {
    #[error("Initialization failed: {0}")]
    InitializationFailed(String),
    #[error("Backend not supported")]
    BackendNotSupported,
    #[error("Device creation failed: {0}")]
    DeviceCreationFailed(String),
    #[error("Swapchain creation failed: {0}")]
    SwapchainCreationFailed(String),
    #[error("Shader compilation failed: {0}")]
    ShaderCompilationFailed(#[from] ShaderCompilationError),
    #[error("Pipeline creation failed: {0}")]
    PipelineCreationFailed(String),
    #[error("Buffer creation failed: {0}")]
    BufferCreationFailed(String),
    #[error("Command pool creation failed: {0}")]
    CommandPoolCreationFailed(String),
    #[error("Command buffer allocation failed: {0}")]
    CommandBufferAllocationFailed(String),
    #[error("Acquire image failed: {0}")]
    AcquireImageFailed(String),
    #[error("Present failed: {0}")]
    PresentFailed(String),
    #[error("Submit failed: {0}")]
    SubmitFailed(String),
    #[error("Map failed: {0}")]
    MapFailed(String),
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    #[error("Resource already exists: {0}")]
    ResourceAlreadyExists(String),
    #[error("Invalid resource state: {0}")]
    InvalidResourceState(String),
    #[error("Validation error: {0}")]
    ValidationError(String),
    #[error("Out of memory: {0}")]
    OutOfMemory(String),
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("Timeout: {0}")]
    Timeout(String),
    #[error("Driver error: {0}")]
    DriverError(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ShaderCompilationError {
    #[error("Invalid shader code: {0}")]
    InvalidShaderCode(String),
    #[error("Unsupported shader stage: {0}")]
    UnsupportedShaderStage(String),
    #[error("Compilation error: {0}")]
    CompilationError(String),
}

impl From<String> for RhiError {
    fn from(s: String) -> Self {
        RhiError::InitializationFailed(s)
    }
}

impl From<&str> for RhiError {
    fn from(s: &str) -> Self {
        RhiError::InitializationFailed(s.to_string())
    }
}

impl From<std::io::Error> for RhiError {
    fn from(err: std::io::Error) -> Self {
        RhiError::InitializationFailed(err.to_string())
    }
}

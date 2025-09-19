use thiserror::Error;

/// Error type for parsing Feature from string
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("invalid feature string")]
pub struct FeatureParseError;

/// General RHI error type
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RHIError {
    #[error("Operation failed")]
    Failed,
    
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    
    #[error("Out of memory")]
    OutOfMemory,
    
    #[error("Device lost")]
    DeviceLost,
    
    #[error("Not supported")]
    NotSupported,
    
    #[error("Resource not found")]
    NotFound,
    
    #[error("Timeout occurred")]
    Timeout,
    
    #[error("Feature parse error")]
    FeatureParse(#[from] FeatureParseError),
    
    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl RHIError {
    pub fn failed() -> Self {
        Self::Failed
    }
    
    pub fn invalid_arg(msg: impl Into<String>) -> Self {
        Self::InvalidArgument(msg.into())
    }
    
    pub fn unknown(msg: impl Into<String>) -> Self {
        Self::Unknown(msg.into())
    }
}

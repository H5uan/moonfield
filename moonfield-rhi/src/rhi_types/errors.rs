use std::sync::Arc;

use thiserror::Error;

/// Error type for parsing Feature from string
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("invalid feature string")]
pub struct FeatureParseError;

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct InstanceError {
    message: String,

    #[source]
    source: Option<Arc<dyn std::error::Error + Send + Sync + 'static>>,
}

impl InstanceError {
    pub fn new(message: String) -> Self {
        Self { message, source: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum DeviceError {
    #[error("Out of memory")]
    OutOfMemory,
    #[error("Device is lost")]
    DeviceLost,
    #[error("Unexpected error variant (driver implementation is at fault)")]
    Unexpected,
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum SurfaceError {
    #[error("Surface is lost")]
    Lost,
    #[error("Surface is outdated, needs to be re-created")]
    Outdated,
    #[error(transparent)]
    Device(#[from] DeviceError),
    #[error("Other reason: {0}")]
    Other(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum Shadererror {
    #[error("Compilation failed: {0:?}")]
    Compilation(String),
    #[error(transparent)]
    Device(#[from] DeviceError),
}

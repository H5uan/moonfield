use thiserror::Error;

/// Error type for parsing Feature from string
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("invalid feature string")]
pub struct FeatureParseError;

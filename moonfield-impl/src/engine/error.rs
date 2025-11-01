use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    /// Internal error
    #[error("{0}")]
    Custom(String),
}

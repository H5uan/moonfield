//! Shared types for Moonfield RHI

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorType {
    Internal,
    OutOfMemory,
    Validation,
    DeviceLost,
}

pub trait MoonfieldRhiError: core::error::Error + 'static {
    fn error_type(&self) -> ErrorType;
}

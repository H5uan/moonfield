use std::any::Any;

use crate::types::RhiError;

/// Trait for buffer functionality
pub trait Buffer: Any {
    /// Maps the buffer memory to CPU-accessible memory
    fn map(&self) -> Result<*mut u8, RhiError>;
    
    /// Unmaps the buffer memory
    fn unmap(&self);
}
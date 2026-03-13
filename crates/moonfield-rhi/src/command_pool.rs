use std::sync::Arc;

use crate::types::RhiError;
use crate::CommandBuffer;

/// Trait for command pool functionality
pub trait CommandPool {
    /// Allocates a new command buffer from the pool
    fn allocate_command_buffer(&self) -> Result<Arc<dyn CommandBuffer>, RhiError>;
}
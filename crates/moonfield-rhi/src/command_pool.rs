use std::sync::Arc;

use crate::CommandBuffer;
use crate::types::RhiError;

/// Trait for command pool functionality
pub trait CommandPool {
    /// Allocates a new command buffer from the pool
    fn allocate_command_buffer(
        &self,
    ) -> Result<Arc<dyn CommandBuffer>, RhiError>;
}

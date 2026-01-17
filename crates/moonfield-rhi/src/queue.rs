use std::sync::Arc;

use crate::CommandBuffer;
use crate::types::RhiError;

/// Trait for queue functionality
pub trait Queue {
    /// Submits command buffers to the queue
    fn submit(
        &self, 
        command_buffers: &[Arc<dyn CommandBuffer>], 
        wait_semaphore: Option<u64>, 
        signal_semaphore: Option<u64>
    ) -> Result<(), RhiError>;
    
    /// Waits until all commands in the queue have finished executing
    fn wait_idle(&self) -> Result<(), RhiError>;
}
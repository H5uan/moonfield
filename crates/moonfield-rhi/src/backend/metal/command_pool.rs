use crate::{types::*, CommandPool, CommandBuffer, RhiError};
use std::sync::Arc;

// Import tracing for logging
use tracing;

pub struct MetalCommandPool {
    pub queue: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLCommandQueue>>,
}

impl CommandPool for MetalCommandPool {
    fn allocate_command_buffer(&self) -> Result<Arc<dyn CommandBuffer>, RhiError> {
        tracing::debug!("Allocating Metal command buffer");
        unsafe {
            let command_buffer = self
                .queue
                .commandBuffer()
                .ok_or_else(|| {
                    tracing::error!("Failed to allocate Metal command buffer");
                    RhiError::CommandBufferAllocationFailed("Failed to allocate Metal command buffer".to_string())
                })?;

            tracing::info!("Metal command buffer allocated successfully");
            Ok(Arc::new(MetalCommandBuffer {
                command_buffer,
                current_encoder: None,
            }))
        }
    }
}
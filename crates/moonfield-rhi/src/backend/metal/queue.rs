use crate::{types::*, Queue, CommandBuffer, RhiError};
use std::sync::Arc;

// Import tracing for logging
use tracing;

pub struct MetalQueue {
    pub queue: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLCommandQueue>>,
}

impl Queue for MetalQueue {
    fn submit(&self, command_buffers: &[Arc<dyn CommandBuffer>], _wait_semaphore: Option<u64>, _signal_semaphore: Option<u64>) -> Result<(), RhiError> {
        tracing::debug!("Submitting {} command buffers to Metal queue", command_buffers.len());
        for (i, cb) in command_buffers.iter().enumerate() {
            let cb_any = cb.as_ref() as &dyn std::any::Any;
            let metal_cb = cb_any.downcast_ref::<MetalCommandBuffer>().unwrap();
            
            tracing::trace!("Committing command buffer {}", i);
            unsafe {
                metal_cb.command_buffer.commit();
            }
        }
        tracing::debug!("Successfully submitted {} command buffers to Metal queue", command_buffers.len());
        Ok(())
    }

    fn wait_idle(&self) -> Result<(), RhiError> {
        Ok(())
    }
}
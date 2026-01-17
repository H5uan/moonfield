use crate::{types::*, *};
use ash::vk::Handle;
use std::any::Any;
use std::sync::Arc;

use super::{VulkanCommandBuffer};

pub struct VulkanQueue {
    pub device: ash::Device,
    pub queue: ash::vk::Queue,
}

impl Queue for VulkanQueue {
    fn submit(&self, command_buffers: &[Arc<dyn CommandBuffer>], wait_semaphore: Option<u64>, signal_semaphore: Option<u64>) -> Result<(), RhiError> {
        unsafe {
            let vk_command_buffers: Vec<_> = command_buffers
                .iter()
                .map(|cb| {
                    (&**cb as &dyn Any)
                        .downcast_ref::<VulkanCommandBuffer>()
                        .expect("command buffer must be VulkanCommandBuffer")
                        .command_buffer
                })
                .collect();

            let wait_semaphores = wait_semaphore.map(|s| vec![ash::vk::Semaphore::from_raw(s)]).unwrap_or_default();
            let signal_semaphores = signal_semaphore.map(|s| vec![ash::vk::Semaphore::from_raw(s)]).unwrap_or_default();
            let wait_stages = vec![ash::vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];

            let submit_info = ash::vk::SubmitInfo::default()
                .command_buffers(&vk_command_buffers)
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&wait_stages)
                .signal_semaphores(&signal_semaphores);

            self.device
                .queue_submit(self.queue, &[submit_info], ash::vk::Fence::null())
                .map_err(|e| RhiError::SubmitFailed(format!("Failed to submit command buffer to queue: {}", e)))
        }
    }

    fn wait_idle(&self) -> Result<(), RhiError> {
        unsafe {
            self.device
                .queue_wait_idle(self.queue)
                .map_err(|e| RhiError::InitializationFailed(format!("Queue wait idle failed: {}", e)))
        }
    }
}
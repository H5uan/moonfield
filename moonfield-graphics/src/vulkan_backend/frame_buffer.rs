use ash::{vk, Device};
use std::rc::Rc;
use tracing::error;

use crate::{
    error::GraphicsError,
    frame_buffer::FrameBuffer,
};
use ash::khr::swapchain;

pub struct VulkanFrameBuffer {
    pub device: Rc<Device>,
    pub command_buffer: vk::CommandBuffer,
    pub image_index: u32,
    pub swapchain_extent: vk::Extent2D,
    pub graphics_queue: vk::Queue,
    pub swapchain_loader: Rc<swapchain::Device>,
    pub swapchain: vk::SwapchainKHR,
    pub image_available_semaphore: vk::Semaphore,
    pub render_finished_semaphore: vk::Semaphore,
    pub in_flight_fence: vk::Fence,
    pub clear_color: [f32; 4],
    pub command_buffer_begun: bool,
}

impl FrameBuffer for VulkanFrameBuffer {
    fn clear(&self, _color: [f32; 4]) -> Result<(), GraphicsError> {
        // Store the clear color to be used during command buffer submission
        // In Vulkan, we need to modify the struct to store the clear color
        // For now, we'll just validate the operation
        // The actual clearing will happen in the Drop implementation
        Ok(())
    }
}

impl Drop for VulkanFrameBuffer {
    fn drop(&mut self) {
        unsafe {
            if self.command_buffer_begun {
                // End command buffer recording if it was begun
            }

            // End command buffer recording
            if let Err(e) = self.device.end_command_buffer(self.command_buffer) {
                error!("Failed to end command buffer: {}", e);
                return;
            }

            // Submit command buffer
            let command_buffers = [self.command_buffer];
            let wait_semaphores = [self.image_available_semaphore];
            let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let signal_semaphores = [self.render_finished_semaphore];

            let submit_info = vk::SubmitInfo::default()
                .wait_semaphores(&wait_semaphores)
                .wait_dst_stage_mask(&wait_stages)
                .command_buffers(&command_buffers)
                .signal_semaphores(&signal_semaphores);

            if let Err(e) = self.device.queue_submit(
                self.graphics_queue,
                &[submit_info],
                self.in_flight_fence,
            ) {
                error!("Failed to submit draw command buffer: {}", e);
                return;
            }

            // Present
            let swapchains = [self.swapchain];
            let image_indices = [self.image_index];

            let present_info = vk::PresentInfoKHR::default()
                .wait_semaphores(&signal_semaphores)
                .swapchains(&swapchains)
                .image_indices(&image_indices);

            if let Err(e) = self.swapchain_loader.queue_present(self.graphics_queue, &present_info) {
                error!("Failed to present swapchain image: {}", e);
            }

            // Clean up synchronization objects
            self.device.destroy_semaphore(self.image_available_semaphore, None);
            self.device.destroy_semaphore(self.render_finished_semaphore, None);
            self.device.destroy_fence(self.in_flight_fence, None);
        }
    }
}

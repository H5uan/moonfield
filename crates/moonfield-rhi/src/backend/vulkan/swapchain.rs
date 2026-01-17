use crate::{types::*, *};
use ash::vk::Handle;
use std::any::Any;
use std::sync::Arc;

pub struct VulkanSwapchain {
    pub device: ash::Device,
    pub swapchain_loader: ash::khr::swapchain::Device,
    pub swapchain: ash::vk::SwapchainKHR,
    pub images: Vec<ash::vk::Image>,
    pub image_views: Vec<ash::vk::ImageView>,
    pub format: Format,
    pub extent: Extent2D,
    pub image_available_semaphores: Vec<ash::vk::Semaphore>,
    pub render_finished_semaphores: Vec<ash::vk::Semaphore>,
    pub queue: ash::vk::Queue,
    pub current_frame: std::sync::atomic::AtomicUsize,
    pub image_layouts: std::sync::Mutex<Vec<ash::vk::ImageLayout>>,
}

impl Swapchain for VulkanSwapchain {
    fn acquire_next_image(&self) -> Result<SwapchainImage, RhiError> {
        unsafe {
            let frame = self.current_frame.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let semaphore = self.image_available_semaphores[frame % self.image_available_semaphores.len()];
            
            let (index, _) = self
                .swapchain_loader
                .acquire_next_image(
                    self.swapchain,
                    u64::MAX,
                    semaphore,
                    ash::vk::Fence::null(),
                )
                .map_err(|e| RhiError::AcquireImageFailed(format!("Failed to acquire next swapchain image: {}", e)))?;

            Ok(SwapchainImage {
                index,
                image_view: self.image_views[index as usize].as_raw() as usize,
                wait_semaphore: semaphore.as_raw(),
                signal_semaphore: self.render_finished_semaphores[index as usize].as_raw(),
            })
        }
    }

    fn present(&self, image: SwapchainImage) -> Result<(), RhiError> {
        unsafe {
            let swapchains = [self.swapchain];
            let image_indices = [image.index];
            let wait_semaphore = ash::vk::Semaphore::from_raw(image.signal_semaphore);
            let wait_semaphores = [wait_semaphore];

            let present_info = ash::vk::PresentInfoKHR::default()
                .wait_semaphores(&wait_semaphores)
                .swapchains(&swapchains)
                .image_indices(&image_indices);

            self.device.queue_wait_idle(self.queue).ok();

            self.swapchain_loader
                .queue_present(self.queue, &present_info)
                .map_err(|e| RhiError::PresentFailed(format!("Failed to present swapchain image: {}", e)))?;

            Ok(())
        }
    }

    fn get_format(&self) -> Format {
        self.format
    }

    fn get_extent(&self) -> Extent2D {
        self.extent
    }
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            for &image_view in &self.image_views {
                self.device.destroy_image_view(image_view, None);
            }
            for &semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &semaphore in &self.render_finished_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.swapchain_loader.destroy_swapchain(self.swapchain, None);
        }
    }
}
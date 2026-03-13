use std::any::Any;
use std::sync::Arc;
use std::sync::RwLock;

use ash::vk::Handle;

use crate::{types::*, *};

pub struct VulkanSwapchain {
    pub device: ash::Device,
    pub swapchain_loader: ash::khr::swapchain::Device,
    pub surface: ash::vk::SurfaceKHR,
    pub swapchain: RwLock<ash::vk::SwapchainKHR>,
    pub images: RwLock<Vec<ash::vk::Image>>,
    pub image_views: RwLock<Vec<ash::vk::ImageView>>,
    pub format: Format,
    pub extent: RwLock<Extent2D>,
    pub image_available_semaphores: Vec<ash::vk::Semaphore>,
    pub render_finished_semaphores: RwLock<Vec<ash::vk::Semaphore>>,
    pub queue: ash::vk::Queue,
    pub current_frame: std::sync::atomic::AtomicUsize,
    pub image_layouts: std::sync::Mutex<Vec<ash::vk::ImageLayout>>,
}

impl Swapchain for VulkanSwapchain {
    fn acquire_next_image(&self) -> Result<SwapchainImage, RhiError> {
        unsafe {
            let frame = self
                .current_frame
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let semaphore = self.image_available_semaphores
                [frame % self.image_available_semaphores.len()];

            let swapchain = *self.swapchain.read().unwrap();
            let (index, suboptimal) = self
                .swapchain_loader
                .acquire_next_image(
                    swapchain,
                    u64::MAX,
                    semaphore,
                    ash::vk::Fence::null(),
                )
                .map_err(|e| {
                    RhiError::AcquireImageFailed(format!(
                        "Failed to acquire next swapchain image: {}",
                        e
                    ))
                })?;

            // Check if swapchain is suboptimal (needs resize)
            if suboptimal {
                return Err(RhiError::AcquireImageFailed(
                    "Swapchain is suboptimal".to_string(),
                ));
            }

            let image_views = self.image_views.read().unwrap();
            let render_finished =
                self.render_finished_semaphores.read().unwrap();

            Ok(SwapchainImage {
                index,
                image_view: image_views[index as usize].as_raw() as usize,
                wait_semaphore: semaphore.as_raw(),
                signal_semaphore: render_finished[index as usize].as_raw(),
            })
        }
    }

    fn present(&self, image: SwapchainImage) -> Result<(), RhiError> {
        unsafe {
            let swapchain = *self.swapchain.read().unwrap();
            let swapchains = [swapchain];
            let image_indices = [image.index];
            let wait_semaphore =
                ash::vk::Semaphore::from_raw(image.signal_semaphore);
            let wait_semaphores = [wait_semaphore];

            let present_info = ash::vk::PresentInfoKHR::default()
                .wait_semaphores(&wait_semaphores)
                .swapchains(&swapchains)
                .image_indices(&image_indices);

            self.device.queue_wait_idle(self.queue).ok();

            let result =
                self.swapchain_loader.queue_present(self.queue, &present_info);

            match result {
                Ok(_) => Ok(()),
                Err(ash::vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                    Err(RhiError::PresentFailed(
                        "Swapchain out of date".to_string(),
                    ))
                }
                Err(e) => Err(RhiError::PresentFailed(format!(
                    "Failed to present swapchain image: {}",
                    e
                ))),
            }
        }
    }

    fn get_format(&self) -> Format {
        self.format
    }

    fn get_extent(&self) -> Extent2D {
        *self.extent.read().unwrap()
    }

    fn resize(&self, new_extent: Extent2D) -> Result<(), RhiError> {
        unsafe {
            tracing::info!(
                "Resizing swapchain to {}x{}",
                new_extent.width,
                new_extent.height
            );

            // Wait for device to be idle
            self.device.device_wait_idle().map_err(|e| {
                RhiError::SwapchainCreationFailed(format!(
                    "Failed to wait for device idle: {}",
                    e
                ))
            })?;

            // Get old swapchain
            let old_swapchain = *self.swapchain.read().unwrap();

            // Clean up old resources
            {
                let mut image_views = self.image_views.write().unwrap();
                for &image_view in image_views.iter() {
                    self.device.destroy_image_view(image_view, None);
                }
                image_views.clear();
            }

            {
                let mut images = self.images.write().unwrap();
                images.clear();
            }

            {
                let mut render_finished =
                    self.render_finished_semaphores.write().unwrap();
                for &semaphore in render_finished.iter() {
                    self.device.destroy_semaphore(semaphore, None);
                }
                render_finished.clear();
            }

            // Determine format
            let format = match self.format {
                Format::BGRA8Unorm => ash::vk::Format::B8G8R8A8_UNORM,
                Format::BGRA8UnormSrgb => ash::vk::Format::B8G8R8A8_SRGB,
                _ => ash::vk::Format::B8G8R8A8_UNORM,
            };

            // Create new swapchain
            let create_info = ash::vk::SwapchainCreateInfoKHR::default()
                .surface(self.surface)
                .min_image_count(self.image_available_semaphores.len() as u32)
                .image_format(format)
                .image_color_space(ash::vk::ColorSpaceKHR::SRGB_NONLINEAR)
                .image_extent(ash::vk::Extent2D {
                    width: new_extent.width,
                    height: new_extent.height,
                })
                .image_array_layers(1)
                .image_usage(ash::vk::ImageUsageFlags::COLOR_ATTACHMENT)
                .image_sharing_mode(ash::vk::SharingMode::EXCLUSIVE)
                .pre_transform(ash::vk::SurfaceTransformFlagsKHR::IDENTITY)
                .composite_alpha(ash::vk::CompositeAlphaFlagsKHR::OPAQUE)
                .present_mode(ash::vk::PresentModeKHR::FIFO)
                .clipped(true)
                .old_swapchain(old_swapchain);

            let new_swapchain = self
                .swapchain_loader
                .create_swapchain(&create_info, None)
                .map_err(|e| {
                    RhiError::SwapchainCreationFailed(format!(
                        "Failed to create swapchain: {}",
                        e
                    ))
                })?;

            // Get new images
            let new_images = self
                .swapchain_loader
                .get_swapchain_images(new_swapchain)
                .map_err(|e| {
                    RhiError::SwapchainCreationFailed(format!(
                        "Failed to get swapchain images: {}",
                        e
                    ))
                })?;

            // Create new image views
            let new_image_views: Vec<_> = new_images
                .iter()
                .map(|&image| {
                    let create_info = ash::vk::ImageViewCreateInfo::default()
                        .image(image)
                        .view_type(ash::vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .components(ash::vk::ComponentMapping::default())
                        .subresource_range(ash::vk::ImageSubresourceRange {
                            aspect_mask: ash::vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        });

                    self.device
                        .create_image_view(&create_info, None)
                        .expect("Failed to create image view")
                })
                .collect();

            // Create new render finished semaphores
            let semaphore_create_info = ash::vk::SemaphoreCreateInfo::default();
            let new_render_finished: Vec<_> = (0..new_images.len())
                .map(|_| {
                    self.device
                        .create_semaphore(&semaphore_create_info, None)
                        .expect("Failed to create semaphore")
                })
                .collect();

            // Update stored values
            *self.swapchain.write().unwrap() = new_swapchain;
            *self.images.write().unwrap() = new_images;
            *self.image_views.write().unwrap() = new_image_views;
            *self.render_finished_semaphores.write().unwrap() =
                new_render_finished;
            *self.extent.write().unwrap() = new_extent;

            // Reset current frame counter
            self.current_frame.store(0, std::sync::atomic::Ordering::Relaxed);

            // Reset image layouts
            let image_count = self.images.read().unwrap().len();
            *self.image_layouts.lock().unwrap() =
                vec![ash::vk::ImageLayout::UNDEFINED; image_count];

            tracing::info!(
                "Swapchain resized successfully with {} images",
                image_count
            );

            // Destroy old swapchain
            self.swapchain_loader.destroy_swapchain(old_swapchain, None);

            Ok(())
        }
    }
}

impl Drop for VulkanSwapchain {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();

            let image_views = self.image_views.read().unwrap();
            for &image_view in image_views.iter() {
                self.device.destroy_image_view(image_view, None);
            }

            for &semaphore in &self.image_available_semaphores {
                self.device.destroy_semaphore(semaphore, None);
            }

            let render_finished =
                self.render_finished_semaphores.read().unwrap();
            for &semaphore in render_finished.iter() {
                self.device.destroy_semaphore(semaphore, None);
            }

            let swapchain = *self.swapchain.read().unwrap();
            self.swapchain_loader.destroy_swapchain(swapchain, None);
        }
    }
}

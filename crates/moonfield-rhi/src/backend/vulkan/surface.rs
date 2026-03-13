use std::any::Any;
use std::sync::Arc;

use ash::vk::Handle;

use super::VulkanAdapter;
use crate::{types::*, *};

pub struct VulkanSurface {
    /// KHR surface handle
    pub surface: ash::vk::SurfaceKHR,
    /// KHR surface function loader
    pub surface_loader: ash::khr::surface::Instance,
}

impl Surface for VulkanSurface {
    fn get_capabilities(&self, adapter: &dyn Adapter) -> SurfaceCapabilities {
        let vk_adapter = (adapter as &dyn Any)
            .downcast_ref::<VulkanAdapter>()
            .expect("adapter must be VulkanAdapter");

        unsafe {
            let caps = self
                .surface_loader
                .get_physical_device_surface_capabilities(
                    vk_adapter.physical_device,
                    self.surface,
                )
                .unwrap();

            let formats = self
                .surface_loader
                .get_physical_device_surface_formats(
                    vk_adapter.physical_device,
                    self.surface,
                )
                .unwrap();

            SurfaceCapabilities {
                formats: formats
                    .iter()
                    .map(|f| match f.format {
                        ash::vk::Format::B8G8R8A8_UNORM => Format::BGRA8Unorm,
                        ash::vk::Format::B8G8R8A8_SRGB => {
                            Format::BGRA8UnormSrgb
                        }
                        _ => Format::BGRA8Unorm,
                    })
                    .collect(),
                present_modes: vec![PresentMode::Fifo],
                min_image_count: caps.min_image_count,
                max_image_count: caps.max_image_count,
            }
        }
    }
}

impl Drop for VulkanSurface {
    fn drop(&mut self) {
        unsafe {
            self.surface_loader.destroy_surface(self.surface, None);
        }
    }
}

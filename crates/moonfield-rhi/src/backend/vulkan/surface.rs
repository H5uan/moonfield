use crate::{types::*, *};
use ash::vk::Handle;
use std::any::Any;
use std::sync::Arc;

use super::{VulkanAdapter};

pub struct VulkanSurface {
    pub surface: ash::vk::SurfaceKHR,
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
                .get_physical_device_surface_capabilities(vk_adapter.physical_device, self.surface)
                .unwrap();

            let formats = self
                .surface_loader
                .get_physical_device_surface_formats(vk_adapter.physical_device, self.surface)
                .unwrap();

            SurfaceCapabilities {
                formats: formats.iter().map(|f| match f.format {
                    ash::vk::Format::B8G8R8A8_UNORM => Format::B8G8R8A8Unorm,
                    ash::vk::Format::B8G8R8A8_SRGB => Format::B8G8R8A8Srgb,
                    _ => Format::B8G8R8A8Unorm,
                }).collect(),
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
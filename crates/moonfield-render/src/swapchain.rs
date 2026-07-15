//! Vulkan surface and swapchain abstraction.

use crate::device::Device;
use crate::error::{Error, Result};
use crate::instance::Instance;
use ash::vk;
use raw_window_handle::{DisplayHandle, HasDisplayHandle, HasWindowHandle, WindowHandle};

/// A window surface.
pub struct Surface {
    surface: vk::SurfaceKHR,
    surface_instance: ash::khr::surface::Instance,
}

impl Surface {
    /// Create a surface from a raw window and display handle.
    ///
    /// # Safety
    ///
    /// The handles must be valid for the lifetime of the returned `Surface`.
    pub unsafe fn from_handles(
        entry: &ash::Entry,
        instance: &Instance,
        window_handle: WindowHandle,
        display_handle: DisplayHandle,
    ) -> Result<Self> {
        let ash_instance = instance.raw();
        let surface = ash_window::create_surface(
            entry,
            ash_instance,
            display_handle.as_raw(),
            window_handle.as_raw(),
            None,
        )
        .map_err(|e| Error::Backend(format!("failed to create surface: {:?}", e)))?;

        Ok(Self {
            surface,
            surface_instance: ash::khr::surface::Instance::new(entry, ash_instance),
        })
    }

    /// Create a surface from a type that implements [`HasWindowHandle`] and
    /// [`HasDisplayHandle`] (e.g. `winit::window::Window`).
    ///
    /// This is a safe wrapper around [`from_handles`].
    pub fn from_window(
        entry: &ash::Entry,
        instance: &Instance,
        window: &(impl HasWindowHandle + HasDisplayHandle),
    ) -> Result<Self> {
        let window_handle = window
            .window_handle()
            .map_err(|e| Error::Backend(format!("failed to get window handle: {e}")))?;
        let display_handle = window
            .display_handle()
            .map_err(|e| Error::Backend(format!("failed to get display handle: {e}")))?;

        // SAFETY: the handles are valid for the lifetime of the window, which
        // is guaranteed by the caller for the returned Surface.
        unsafe { Self::from_handles(entry, instance, window_handle, display_handle) }
    }

    /// Access the raw surface handle.
    pub fn raw(&self) -> vk::SurfaceKHR {
        self.surface
    }

    /// Query surface capabilities for the given physical device.
    pub fn capabilities(
        &self,
        physical_device: vk::PhysicalDevice,
    ) -> Result<vk::SurfaceCapabilitiesKHR> {
        unsafe {
            self.surface_instance
                .get_physical_device_surface_capabilities(physical_device, self.surface)
                .map_err(|e| {
                    Error::Backend(format!("failed to query surface capabilities: {:?}", e))
                })
        }
    }

    /// Query supported surface formats.
    pub fn formats(
        &self,
        physical_device: vk::PhysicalDevice,
    ) -> Result<Vec<vk::SurfaceFormatKHR>> {
        unsafe {
            self.surface_instance
                .get_physical_device_surface_formats(physical_device, self.surface)
                .map_err(|e| Error::Backend(format!("failed to query surface formats: {:?}", e)))
        }
    }

    /// Query supported present modes.
    pub fn present_modes(
        &self,
        physical_device: vk::PhysicalDevice,
    ) -> Result<Vec<vk::PresentModeKHR>> {
        unsafe {
            self.surface_instance
                .get_physical_device_surface_present_modes(physical_device, self.surface)
                .map_err(|e| Error::Backend(format!("failed to query present modes: {:?}", e)))
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            self.surface_instance.destroy_surface(self.surface, None);
        }
    }
}

/// Vulkan swapchain and its image views.
pub struct Swapchain {
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
    format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
    loader: ash::khr::swapchain::Device,
    device: ash::Device,
}

impl Swapchain {
    /// Create a swapchain for the given surface and window size.
    pub fn new(
        instance: &Instance,
        device: &Device,
        surface: &Surface,
        window_size: [u32; 2],
    ) -> Result<Self> {
        let physical_device = device.physical_device();
        let capabilities = surface.capabilities(physical_device)?;
        let formats = surface.formats(physical_device)?;
        let present_modes = surface.present_modes(physical_device)?;

        if formats.is_empty() {
            return Err(Error::Unsupported);
        }

        let format = formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_UNORM
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .copied()
            .unwrap_or(formats[0]);

        let present_mode = present_modes
            .iter()
            .copied()
            .find(|&mode| mode == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(vk::PresentModeKHR::FIFO);

        let extent = if capabilities.current_extent.width != u32::MAX {
            capabilities.current_extent
        } else {
            vk::Extent2D {
                width: window_size[0].clamp(
                    capabilities.min_image_extent.width,
                    capabilities.max_image_extent.width,
                ),
                height: window_size[1].clamp(
                    capabilities.min_image_extent.height,
                    capabilities.max_image_extent.height,
                ),
            }
        };

        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 && image_count > capabilities.max_image_count {
            image_count = capabilities.max_image_count;
        }

        let indices = device.queue_family_indices();
        let family_indices: Vec<u32> = if indices.graphics != indices.present {
            vec![indices.graphics, indices.present]
        } else {
            vec![]
        };
        let sharing_mode = if family_indices.is_empty() {
            vk::SharingMode::EXCLUSIVE
        } else {
            vk::SharingMode::CONCURRENT
        };

        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface.raw())
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(&family_indices)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        let loader = ash::khr::swapchain::Device::new(instance.raw(), device.raw());
        let swapchain = unsafe { loader.create_swapchain(&create_info, None) }
            .map_err(|e| Error::Backend(format!("failed to create swapchain: {:?}", e)))?;

        let images = unsafe { loader.get_swapchain_images(swapchain) }
            .map_err(|e| Error::Backend(format!("failed to get swapchain images: {:?}", e)))?;

        let image_views: Result<Vec<_>> = images
            .iter()
            .map(|image| {
                let create_info = vk::ImageViewCreateInfo::default()
                    .image(*image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format.format)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .base_mip_level(0)
                            .level_count(1)
                            .base_array_layer(0)
                            .layer_count(1),
                    );
                unsafe {
                    device
                        .raw()
                        .create_image_view(&create_info, None)
                        .map_err(|e| {
                            Error::Backend(format!("failed to create image view: {:?}", e))
                        })
                }
            })
            .collect();
        let image_views = image_views?;

        Ok(Self {
            swapchain,
            images,
            image_views,
            format,
            extent,
            loader,
            device: device.raw().clone(),
        })
    }

    /// Access the raw swapchain handle.
    pub fn raw(&self) -> vk::SwapchainKHR {
        self.swapchain
    }

    /// Access the swapchain images.
    pub fn images(&self) -> &[vk::Image] {
        &self.images
    }

    /// Access the swapchain image views.
    pub fn image_views(&self) -> &[vk::ImageView] {
        &self.image_views
    }

    /// Access the selected surface format.
    pub fn format(&self) -> vk::SurfaceFormatKHR {
        self.format
    }

    /// Access the swapchain extent.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        unsafe {
            for view in self.image_views.drain(..) {
                self.device.destroy_image_view(view, None);
            }
            self.loader.destroy_swapchain(self.swapchain, None);
        }
    }
}

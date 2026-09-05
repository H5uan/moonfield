//! Vulkan surface and swapchain abstraction.

use crate::error::{Error, Result};
use crate::types::{Extent2d, Format};
use crate::vulkan::device::Device;
use crate::vulkan::instance::Instance;
use crate::vulkan::sync::Semaphore;
use ash::vk;
use raw_window_handle::{DisplayHandle, HasDisplayHandle, HasWindowHandle, WindowHandle};

/// A window surface.
pub struct Surface {
    surface: vk::SurfaceKHR,
    surface_instance: ash::khr::surface::Instance,
    capabilities2: ash::khr::get_surface_capabilities2::Instance,
}

impl Surface {
    /// Create a surface from a raw window and display handle.
    ///
    /// # Safety
    ///
    /// The handles must be valid for the lifetime of the returned `Surface`.
    pub unsafe fn from_handles(
        instance: &Instance,
        window_handle: WindowHandle,
        display_handle: DisplayHandle,
    ) -> Result<Self> {
        let entry = instance.entry();
        let ash_instance = instance.raw();
        let surface_factory =
            ash_window::SurfaceFactory::new(entry, ash_instance, display_handle.as_raw()).map_err(
                |e| Error::Backend(format!("failed to load surface extension: {:?}", e)),
            )?;
        let surface = unsafe { surface_factory.create_surface(window_handle.as_raw(), None) }
            .map_err(|e| Error::Backend(format!("failed to create surface: {:?}", e)))?;

        Ok(Self {
            surface,
            surface_instance: ash::khr::surface::Instance::load(entry, ash_instance),
            capabilities2: ash::khr::get_surface_capabilities2::Instance::load(entry, ash_instance),
        })
    }

    /// Create a surface from a type that implements [`HasWindowHandle`] and
    /// [`HasDisplayHandle`] (e.g. `winit::window::Window`).
    ///
    /// This is a safe wrapper around [`from_handles`](Self::from_handles).
    pub fn from_window(
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
        unsafe { Self::from_handles(instance, window_handle, display_handle) }
    }

    /// Access the raw surface handle.
    pub(crate) fn raw(&self) -> vk::SurfaceKHR {
        self.surface
    }

    /// Query surface capabilities for the given physical device (Vulkan
    /// 1.1+ "2" query; extended structures can be attached through pNext).
    pub(crate) fn capabilities(
        &self,
        physical_device: vk::PhysicalDevice,
    ) -> Result<vk::SurfaceCapabilities2KHR<'_>> {
        let surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default().surface(self.surface);
        let mut caps = vk::SurfaceCapabilities2KHR::default();
        unsafe {
            self.capabilities2
                .get_physical_device_surface_capabilities2(
                    physical_device,
                    &surface_info,
                    &mut caps,
                )
                .map_err(|e| {
                    Error::Backend(format!("failed to query surface capabilities: {:?}", e))
                })?;
        }
        Ok(caps)
    }

    /// Query supported surface formats (Vulkan "2" query); each entry's base
    /// data is in the `.surface_format` field.
    pub(crate) fn formats(
        &self,
        physical_device: vk::PhysicalDevice,
    ) -> Result<Vec<vk::SurfaceFormat2KHR<'_>>> {
        let surface_info = vk::PhysicalDeviceSurfaceInfo2KHR::default().surface(self.surface);
        let len = unsafe {
            self.capabilities2
                .get_physical_device_surface_formats2_len(physical_device, &surface_info)
        }
        .map_err(|e| Error::Backend(format!("failed to query surface formats: {:?}", e)))?;
        let mut out = vec![vk::SurfaceFormat2KHR::default(); len];
        unsafe {
            self.capabilities2
                .get_physical_device_surface_formats2(physical_device, &surface_info, &mut out)
                .map_err(|e| Error::Backend(format!("failed to query surface formats: {:?}", e)))?;
        }
        Ok(out)
    }

    /// Query supported present modes.
    pub(crate) fn present_modes(
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
    image_views: Vec<vk::ImageView>,
    format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
    loader: ash::khr::swapchain::Device,
    device: ash::Device,
}

impl Swapchain {
    /// Create a swapchain for the given surface and window size.
    ///
    /// For recreation on resize/suboptimal, use [`recreate`](Self::recreate),
    /// which passes the current swapchain as `oldSwapchain` so the driver can
    /// recycle the surface's images.
    pub fn new(
        instance: &Instance,
        device: &Device,
        surface: &Surface,
        window_size: [u32; 2],
    ) -> Result<Self> {
        Self::create(instance, device, surface, window_size, None)
    }

    /// `new` with an `oldSwapchain` handle for the driver to recycle.
    /// Crate-internal: callers go through [`new`](Self::new) or
    /// [`recreate`](Self::recreate).
    pub(crate) fn create(
        instance: &Instance,
        device: &Device,
        surface: &Surface,
        window_size: [u32; 2],
        old_swapchain: Option<vk::SwapchainKHR>,
    ) -> Result<Self> {
        let physical_device = device.physical_device();
        let capabilities = surface.capabilities(physical_device)?;
        let caps = capabilities.surface_capabilities;
        let formats = surface.formats(physical_device)?;
        let present_modes = surface.present_modes(physical_device)?;

        if formats.is_empty() {
            return Err(Error::Unsupported("no surface formats".to_string()));
        }

        let surface_format = formats
            .iter()
            .find(|f| {
                f.surface_format.format == vk::Format::B8G8R8A8_UNORM
                    && f.surface_format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .map(|f| f.surface_format)
            .unwrap_or(formats[0].surface_format);

        let present_mode = present_modes
            .iter()
            .copied()
            .find(|&mode| mode == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(vk::PresentModeKHR::FIFO);

        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: window_size[0]
                    .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: window_size[1]
                    .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };

        let mut image_count = caps.min_image_count + 1;
        if caps.max_image_count > 0 && image_count > caps.max_image_count {
            image_count = caps.max_image_count;
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
            .old_swapchain(old_swapchain.unwrap_or(vk::SwapchainKHR::null()))
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(sharing_mode)
            .queue_family_indices(&family_indices)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);

        let loader = ash::khr::swapchain::Device::load(instance.raw(), device.raw());
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
                    .format(surface_format.format)
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
            image_views,
            format: surface_format,
            extent,
            loader,
            device: device.raw().clone(),
        })
    }

    /// Recreate the swapchain for a new window size.
    ///
    /// The current swapchain is passed to the driver as `oldSwapchain` so it
    /// can recycle the surface's images; the old swapchain is dropped after
    /// the new one is created. The caller must ensure the device is idle.
    pub fn recreate(
        &mut self,
        instance: &Instance,
        device: &Device,
        surface: &Surface,
        window_size: [u32; 2],
    ) -> Result<()> {
        let old_swapchain = self.swapchain;
        // The replacement is built before `self` is overwritten, so the old
        // swapchain is dropped (destroyed) only after the new one exists.
        *self = Self::create(instance, device, surface, window_size, Some(old_swapchain))?;
        Ok(())
    }

    /// Borrow the image view of swapchain image `index` (valid only between
    /// acquire and present of that image).
    pub fn image_view(&self, index: u32) -> crate::vulkan::view::TextureView {
        crate::vulkan::view::TextureView::borrow_raw(
            self.image_views[index as usize],
            self.device.clone(),
        )
    }

    /// Access the swapchain extent, in the crate's vocabulary.
    pub fn extent(&self) -> Extent2d {
        Extent2d {
            width: self.extent.width,
            height: self.extent.height,
        }
    }

    /// Acquire the next available swapchain image.
    ///
    /// Returns `(image_index, suboptimal)` on success. An out-of-date
    /// swapchain maps to [`Error::SurfaceOutOfDate`]; a suboptimal result
    /// still returns the image with `suboptimal == true`.
    pub fn acquire_next_image(
        &self,
        timeout_ns: u64,
        semaphore: &Semaphore,
    ) -> Result<(u32, bool)> {
        // SAFETY: the swapchain and semaphore are valid handles; no fence is
        // used, the caller synchronizes with its own in-flight fence.
        unsafe {
            self.loader.acquire_next_image(
                self.swapchain,
                timeout_ns,
                semaphore.raw(),
                vk::Fence::null(),
            )
        }
        .map_err(|result| match result {
            vk::Result::ERROR_OUT_OF_DATE_KHR => Error::SurfaceOutOfDate,
            other => Error::from_vk(other),
        })
    }

    /// Present a rendered swapchain image, waiting on the given semaphores.
    ///
    /// Returns `true` when the swapchain is suboptimal for the surface. An
    /// out-of-date swapchain maps to [`Error::SurfaceOutOfDate`].
    pub fn queue_present(
        &self,
        device: &Device,
        wait_semaphores: &[&Semaphore],
        image_index: u32,
    ) -> Result<bool> {
        let swapchains = [self.swapchain];
        let image_indices = [image_index];
        let wait: Vec<vk::Semaphore> = wait_semaphores.iter().map(|s| s.raw()).collect();
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&wait)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        // SAFETY: the swapchain, queue and semaphores are valid handles; the
        // image index was acquired from this swapchain.
        unsafe {
            self.loader
                .queue_present(device.present_queue(), &present_info)
        }
        .map_err(|result| match result {
            vk::Result::ERROR_OUT_OF_DATE_KHR => Error::SurfaceOutOfDate,
            other => Error::from_vk(other),
        })
    }

    /// The swapchain color format in the crate's vocabulary, plus whether the
    /// framebuffer is sRGB-encoded (an sRGB target needs the UI shader's
    /// linearizing fragment entry).
    pub fn format_srgb(&self) -> Result<(Format, bool)> {
        match self.format.format {
            vk::Format::B8G8R8A8_UNORM => Ok((Format::B8G8R8A8Unorm, false)),
            vk::Format::R8G8B8A8_UNORM => Ok((Format::R8G8B8A8Unorm, false)),
            vk::Format::B8G8R8A8_SRGB => Ok((Format::B8G8R8A8Unorm, true)),
            vk::Format::R8G8B8A8_SRGB => Ok((Format::R8G8B8A8Unorm, true)),
            other => Err(Error::Backend(format!(
                "unsupported swapchain format {other:?} (expected BGRA/RGBA unorm/sRGB)"
            ))),
        }
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

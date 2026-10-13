//! Vulkan surface and swapchain abstraction.

use super::device::{Device, DeviceContext};
use super::formats::from_vk;
use super::instance::{Instance, InstanceShared};
use super::sync::Semaphore;
use crate::RETIRE_RING;
use crate::error::{Error, Result};
use crate::types::{Extent2d, Format};
use ash::vk;
use raw_window_handle::{DisplayHandle, HasDisplayHandle, HasWindowHandle, WindowHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A window surface.
pub struct Surface {
    surface: vk::SurfaceKHR,
    surface_instance: ash::khr::surface::Instance,
    capabilities2: ash::khr::get_surface_capabilities2::Instance,
    /// Keepalive: destroying a surface requires a live instance, so the
    /// surface holds one rather than relying on the caller's drop order.
    _instance: Arc<InstanceShared>,
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
            _instance: instance.shared(),
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
        // SAFETY: the surface was created from this instance and is destroyed
        // exactly once, here; `_instance` keeps the instance alive.
        unsafe {
            self.surface_instance.destroy_surface(self.surface, None);
        }
    }
}

/// One frame slot's acquire/present semaphore pair, owned by the swapchain.
/// An [`AcquiredImage`] clones the two semaphore `Arc`s so
/// [`Device::submit_frame`](crate::Device::submit_frame) and
/// [`Swapchain::present`] reach the slot's semaphores without borrowing the
/// swapchain.
struct FrameSlot {
    /// Signaled by [`Swapchain::acquire`], waited by the frame's submit. In
    /// an `Arc` (rather than sharing the whole slot) so the abort path's
    /// lazy rebuild can swap in a fresh semaphore through plain `&mut`
    /// access — the rebuild runs only when no `AcquiredImage` references the
    /// slot.
    image_available: Arc<Semaphore>,
    /// Signaled by the frame's submit, waited by [`Swapchain::present`].
    /// Never rebuilt: an aborted frame's failed submit enqueued nothing, so
    /// it was never signaled by the abort, and its last live signal was
    /// consumed by that frame's present.
    render_finished: Arc<Semaphore>,
    /// Set by [`AcquiredImage::drop`] when the image dies unpresented (an
    /// aborted frame): the acquire's signal on `image_available` then has no
    /// consumer, and a binary semaphore cannot be unsignaled — the next
    /// `acquire` on this slot rebuilds the semaphore before acquiring.
    pending_unconsumed: Arc<AtomicBool>,
}

impl FrameSlot {
    fn new(device: &Device) -> Result<Self> {
        Ok(Self {
            image_available: Arc::new(Semaphore::new(device)?),
            render_finished: Arc::new(Semaphore::new(device)?),
            pending_unconsumed: Arc::new(AtomicBool::new(false)),
        })
    }
}

/// A swapchain image acquisition. [`Swapchain::acquire`] returns it after
/// signaling the frame slot's acquire semaphore; the frame's
/// [`Device::submit_frame`](crate::Device::submit_frame) waits on that signal
/// and signals the slot's render-finished, and [`Swapchain::present`]
/// consumes the acquisition. Dropping an unpresented `AcquiredImage` is an
/// abort (see its `Drop`).
///
/// Contract: an image whose frame was successfully submitted must be
/// presented — dropping it then instead would leave the slot's
/// render-finished signaled with no consumer.
pub struct AcquiredImage {
    index: u32,
    suboptimal: bool,
    image_available: Arc<Semaphore>,
    render_finished: Arc<Semaphore>,
    pending_unconsumed: Arc<AtomicBool>,
    /// Shared with the owning swapchain; set on an abort drop so
    /// [`Swapchain::take_aborted`] can flag recreation.
    aborted: Arc<AtomicBool>,
    /// Set by [`Swapchain::present`]: the acquisition was consumed (however
    /// the present turned out), so `Drop` is not an abort.
    presented: bool,
}

impl AcquiredImage {
    /// The swapchain image index (for [`Swapchain::image_view`]).
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Whether the swapchain is suboptimal for the surface. The image is
    /// still usable: present it and let the driver scale, and let the frame
    /// loop recreate the swapchain at the next tick.
    pub fn suboptimal(&self) -> bool {
        self.suboptimal
    }

    /// The slot's acquire semaphore — what the frame's submit waits on.
    pub(crate) fn acquire_semaphore(&self) -> &Semaphore {
        &self.image_available
    }

    /// The slot's render-finished semaphore — what the frame's submit
    /// signals and [`Swapchain::present`] waits on.
    pub(crate) fn render_finished_semaphore(&self) -> &Semaphore {
        &self.render_finished
    }
}

impl Drop for AcquiredImage {
    fn drop(&mut self) {
        if self.presented {
            return;
        }
        // Dropped unpresented: the frame was aborted before or at its
        // submit, so the acquire's signal on the slot's acquire semaphore
        // has no consumer (a failed queue_submit enqueues nothing). A binary
        // semaphore cannot be unsignaled, so flag the slot — the next
        // `acquire` on it rebuilds the semaphore lazily — and flag the
        // swapchain, whose `take_aborted` tells the frame loop to recreate
        // the swapchain. All accessors are on the frame thread; the atomics
        // only guard the `Arc`-shared state formally.
        self.pending_unconsumed.store(true, Ordering::Release);
        self.aborted.store(true, Ordering::Release);
    }
}

/// Vulkan swapchain and its image views.
pub struct Swapchain {
    swapchain: vk::SwapchainKHR,
    image_views: Vec<vk::ImageView>,
    format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
    loader: ash::khr::swapchain::Device,
    /// The queue [`present`](Self::present) submits to, captured at creation.
    present_queue: vk::Queue,
    /// Per-frame-slot acquire/present semaphore pairs, [`RETIRE_RING`] of
    /// them — one per frames-in-flight slot.
    slots: Vec<FrameSlot>,
    /// Set by an [`AcquiredImage`] abort drop; read and cleared by
    /// [`take_aborted`](Self::take_aborted).
    aborted: Arc<AtomicBool>,
    /// Keeps the device alive until the swapchain and its image views are
    /// destroyed in `Drop`.
    ctx: DeviceContext,
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

        let mut slots = Vec::with_capacity(RETIRE_RING);
        for _ in 0..RETIRE_RING {
            slots.push(FrameSlot::new(device)?);
        }

        Ok(Self {
            swapchain,
            image_views,
            format: surface_format,
            extent,
            loader,
            present_queue: device.present_queue(),
            slots,
            aborted: Arc::new(AtomicBool::new(false)),
            ctx: device.context(),
        })
    }

    /// Create the successor of `old` on the same surface.
    ///
    /// `old`'s handle is passed as `oldSwapchain` so the driver can recycle
    /// the surface's images; a native window may have only one *non-retired*
    /// swapchain, so replacing a swapchain that must stay alive (e.g. for
    /// deferred destruction) has to go through here, not [`new`](Self::new).
    /// The call retires `old` — even if creation fails: it may no longer be
    /// acquired from, images already acquired from it may still be
    /// presented, and ownership stays with the caller, who destroys it by
    /// dropping it once no in-flight frame can still present it.
    pub fn succeed(
        instance: &Instance,
        device: &Device,
        surface: &Surface,
        window_size: [u32; 2],
        old: &Swapchain,
    ) -> Result<Self> {
        Self::create(instance, device, surface, window_size, Some(old.swapchain))
    }

    /// Recreate the swapchain for a new window size.
    ///
    /// The current swapchain is passed to the driver as `oldSwapchain` so it
    /// can recycle the surface's images; the old swapchain is dropped after
    /// the new one is created. The caller must ensure the device is idle.
    /// For recreation that keeps the old swapchain alive (deferred
    /// destruction), use [`succeed`](Self::succeed) instead.
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
    pub fn image_view(&self, index: u32) -> super::view::TextureView {
        let (format, _) = self
            .format_srgb()
            .expect("the swapchain format was validated at creation");
        super::view::TextureView::borrow_raw(
            self.image_views[index as usize],
            self.ctx.clone(),
            self.extent.width,
            self.extent.height,
            format,
        )
    }

    /// Access the swapchain extent, in the crate's vocabulary.
    pub fn extent(&self) -> Extent2d {
        Extent2d {
            width: self.extent.width,
            height: self.extent.height,
        }
    }

    /// Acquire the next swapchain image for frame slot `slot` (0..
    /// [`RETIRE_RING`]), signaling the slot's acquire semaphore. The frame's
    /// [`Device::submit_frame`](crate::Device::submit_frame) waits on that
    /// signal when the returned [`AcquiredImage`] is listed in its
    /// `acquired`.
    ///
    /// An out-of-date swapchain maps to [`Error::SurfaceOutOfDate`]; a
    /// suboptimal result still yields an image
    /// ([`AcquiredImage::suboptimal`]). On an error nothing was signaled.
    ///
    /// Abort repair: if the slot's previous acquisition was dropped
    /// unpresented, its acquire semaphore was left signaled with no consumer
    /// (a binary semaphore cannot be unsignaled), so a fresh one is swapped
    /// in before acquiring.
    pub fn acquire(&mut self, slot: usize, timeout_ns: u64) -> Result<AcquiredImage> {
        let Some(frame_slot) = self.slots.get_mut(slot) else {
            return Err(Error::Validation(format!(
                "frame slot {slot} is out of range ({} slots)",
                self.slots.len()
            )));
        };
        if frame_slot.pending_unconsumed.swap(false, Ordering::AcqRel) {
            frame_slot.image_available = Arc::new(Semaphore::new_ctx(self.ctx.clone())?);
        }
        // SAFETY: the swapchain and the slot's semaphore are valid handles;
        // no fence is used — the frame loop waits the device timeline before
        // reusing a slot, so the previous acquire on it completed.
        let (index, suboptimal) = unsafe {
            self.loader.acquire_next_image(
                self.swapchain,
                timeout_ns,
                frame_slot.image_available.raw(),
                vk::Fence::null(),
            )
        }
        .map_err(|result| match result {
            vk::Result::ERROR_OUT_OF_DATE_KHR => Error::SurfaceOutOfDate,
            other => from_vk(other),
        })?;
        Ok(AcquiredImage {
            index,
            suboptimal,
            image_available: frame_slot.image_available.clone(),
            render_finished: frame_slot.render_finished.clone(),
            pending_unconsumed: frame_slot.pending_unconsumed.clone(),
            aborted: self.aborted.clone(),
            presented: false,
        })
    }

    /// Present an acquired image, waiting on its slot's render-finished
    /// signal (signaled by the frame's
    /// [`Device::submit_frame`](crate::Device::submit_frame)).
    ///
    /// Returns `true` when the swapchain is suboptimal for the surface. An
    /// out-of-date swapchain maps to [`Error::SurfaceOutOfDate`]. The image
    /// is consumed either way: by contract the frame's submit already
    /// consumed the slot's acquire semaphore, so no abort repair applies.
    pub fn present(&mut self, mut image: AcquiredImage) -> Result<bool> {
        image.presented = true;
        let swapchains = [self.swapchain];
        let image_indices = [image.index];
        let wait = [image.render_finished.raw()];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&wait)
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        // SAFETY: the swapchain, queue, and the slot's render-finished
        // semaphore are valid handles; the image index was acquired from
        // this swapchain.
        unsafe { self.loader.queue_present(self.present_queue, &present_info) }.map_err(|result| {
            match result {
                vk::Result::ERROR_OUT_OF_DATE_KHR => Error::SurfaceOutOfDate,
                other => from_vk(other),
            }
        })
    }

    /// Whether an acquired image was dropped unpresented (a frame abort)
    /// since the last call; clears the flag. The frame loop folds this into
    /// its swapchain-recreation flag: the abort leaves the slot's acquire
    /// semaphore signaled with no consumer, and recreation replaces the slot
    /// pairs wholesale (the affected slot also repairs itself lazily on the
    /// next [`acquire`](Self::acquire)).
    pub fn take_aborted(&mut self) -> bool {
        self.aborted.swap(false, Ordering::AcqRel)
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
        // SAFETY: the swapchain and its views were created by this device and
        // are destroyed exactly once, here; `ctx` keeps the device alive.
        unsafe {
            for view in self.image_views.drain(..) {
                self.ctx.raw().destroy_image_view(view, None);
            }
            self.loader.destroy_swapchain(self.swapchain, None);
        }
    }
}

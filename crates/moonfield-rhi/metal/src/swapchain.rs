//! CAMetalLayer-backed surface and swapchain.
//!
//! A [`Surface`] owns the layer; a [`Swapchain`] acquires drawables from it
//! and presents them. Metal has no pre-created image set: each acquire
//! returns the layer's next drawable (a blocking call), and presentation
//! hands the drawable back after the queue's committed work —
//! `signalDrawable` on the queue makes the ordering explicit.

use objc2::ClassType;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CGSize;
use objc2_metal::{MTL4CommandQueue, MTLDrawable};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};

use crate::device::Device;
use crate::formats::ToMetal;
use crate::instance::Instance;
use crate::sync::Semaphore;
use crate::view::TextureView;
use moonfield_rhi_core::{Error, Extent2d, Format, Result};

/// A presentation target: a `CAMetalLayer`. Created standalone (headless,
/// for tests and offscreen rendering) or attached to a window's view.
pub struct Surface {
    layer: Retained<CAMetalLayer>,
}

impl Surface {
    /// A layer not attached to any view — same drawable machinery as a
    /// windowed surface, for headless tests and offscreen rendering. The
    /// layer is created readback-capable (`framebufferOnly` off) so tests
    /// can read pixels; windowed layers keep the default for performance.
    pub fn new_layer(device: &Device, size: [f64; 2]) -> Self {
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(device.ctx().shared().device()));
        layer.setPixelFormat(Format::B8G8R8A8Unorm.to_metal());
        layer.setDrawableSize(CGSize {
            width: size[0],
            height: size[1],
        });
        layer.setFramebufferOnly(false);
        Self { layer }
    }

    /// Create the surface for a window: attach a `CAMetalLayer` to the
    /// window's view (AppKit only).
    ///
    /// The Metal device is global, so `instance` only gates the platform
    /// (Metal 4 family) and provides the device to bind to the layer.
    pub fn from_window(
        instance: &Instance,
        window: &(impl HasWindowHandle + HasDisplayHandle),
    ) -> Result<Self> {
        let handle = window
            .window_handle()
            .map_err(|e| Error::Backend(format!("failed to get window handle: {e}")))?;
        // The display handle is not needed on AppKit; probe it for the
        // caller-contract symmetry with the Vulkan backend.
        window
            .display_handle()
            .map_err(|e| Error::Backend(format!("failed to get display handle: {e}")))?;

        let ns_view = match handle.as_raw() {
            RawWindowHandle::AppKit(view) => view.ns_view,
            other => {
                return Err(Error::Unsupported(format!(
                    "window handle {other:?} is not an AppKit view"
                )));
            }
        };

        // SAFETY: the view pointer comes from a live window that the caller
        // keeps alive for the returned surface (the same contract the
        // Vulkan backend rests on); the view outlives the attached layer.
        let view: &objc2_app_kit::NSView =
            unsafe { &*(ns_view.as_ptr() as *const objc2_app_kit::NSView) };

        let layer = CAMetalLayer::new();
        layer.setDevice(Some(instance.device()));
        layer.setPixelFormat(Format::B8G8R8A8Unorm.to_metal());
        view.setWantsLayer(true);
        // SAFETY: replacing the view's layer while no rendering is in
        // flight; the surface owns the layer from here on.
        view.setLayer(Some(layer.as_super()));

        Ok(Self { layer })
    }

    pub(crate) fn layer(&self) -> &Retained<CAMetalLayer> {
        &self.layer
    }
}

/// A drawable stream over a [`Surface`]'s layer.
///
/// Metal has no pre-created image set to index: [`acquire_next_image`]
/// returns the layer's next drawable (index is always 0) and stores it;
/// [`queue_present`] signals it on the queue and presents. Resize adjusts
/// the layer's drawable size; the next acquire picks it up.
///
/// [`acquire_next_image`]: Self::acquire_next_image
/// [`queue_present`]: Self::queue_present
pub struct Swapchain {
    layer: Retained<CAMetalLayer>,
    extent: Extent2d,
    format: Format,
    drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
}

impl Swapchain {
    /// Bind a swapchain to the surface's layer at `window_size`.
    pub fn new(
        _instance: &Instance,
        _device: &Device,
        surface: &Surface,
        window_size: [u32; 2],
    ) -> Result<Self> {
        let layer = surface.layer();
        layer.setDrawableSize(CGSize {
            width: window_size[0] as f64,
            height: window_size[1] as f64,
        });
        Ok(Self {
            layer: layer.clone(),
            extent: Extent2d {
                width: window_size[0],
                height: window_size[1],
            },
            format: Format::B8G8R8A8Unorm,
            drawable: None,
        })
    }

    /// `new` keeping the old swapchain's layer alive during the swap; Metal
    /// shares the layer, so nothing is recycled — the signature matches the
    /// Vulkan backend's for the engine layer's recreate path.
    pub fn succeed(
        instance: &Instance,
        device: &Device,
        surface: &Surface,
        window_size: [u32; 2],
        _old: &Swapchain,
    ) -> Result<Self> {
        Self::new(instance, device, surface, window_size)
    }

    /// Adjust the drawable size for a new window size. The next acquire
    /// returns a drawable of the new size.
    pub fn recreate(
        &mut self,
        _instance: &Instance,
        _device: &Device,
        _surface: &Surface,
        window_size: [u32; 2],
    ) -> Result<()> {
        self.layer.setDrawableSize(CGSize {
            width: window_size[0] as f64,
            height: window_size[1] as f64,
        });
        self.extent = Extent2d {
            width: window_size[0],
            height: window_size[1],
        };
        self.drawable = None;
        Ok(())
    }

    /// The drawable extent, in the crate's vocabulary.
    pub fn extent(&self) -> Extent2d {
        self.extent
    }

    /// The swapchain color format plus whether the framebuffer is
    /// sRGB-encoded (`CAMetalLayer` defaults to linear BGRA8).
    pub fn format_srgb(&self) -> Result<(Format, bool)> {
        Ok((self.format, false))
    }

    /// A view of the acquired drawable's texture (valid only between
    /// acquire and present). The index is always 0 on this backend.
    pub fn image_view(&self, _index: u32) -> TextureView {
        let drawable = self
            .drawable
            .as_ref()
            .expect("image_view requires an acquired image");
        let texture = drawable.texture();
        TextureView::new(texture, self.extent.width, self.extent.height, self.format)
    }

    /// Acquire the next drawable. Blocking (the layer hands drawables out
    /// when ready); the semaphore parameter matches the Vulkan surface and
    /// carries no GPU signal on this backend.
    ///
    /// Returns `(0, false)` on success; a layer that cannot produce a
    /// drawable maps to [`Error::SurfaceOutOfDate`].
    pub fn acquire_next_image(
        &mut self,
        _timeout_ns: u64,
        _semaphore: &Semaphore,
    ) -> Result<(u32, bool)> {
        let drawable = self.layer.nextDrawable().ok_or(Error::SurfaceOutOfDate)?;
        self.drawable = Some(drawable);
        Ok((0, false))
    }

    /// Present the acquired drawable, after the queue's committed work
    /// (`signalDrawable` orders the queue behind the drawable's
    /// presentation). Wait semaphores match the Vulkan surface; Metal needs
    /// none — the queue's submission order is the ordering.
    ///
    /// Returns `false` (never suboptimal on this backend).
    pub fn queue_present(
        &mut self,
        device: &Device,
        _wait_semaphores: &[&Semaphore],
        _image_index: u32,
    ) -> Result<bool> {
        let drawable = self
            .drawable
            .take()
            .ok_or_else(|| Error::Backend("present without an acquired image".into()))?;
        let drawable: &ProtocolObject<dyn CAMetalDrawable> = &drawable;
        // The queue signal takes the base drawable view.
        let drawable: &ProtocolObject<dyn MTLDrawable> = ProtocolObject::from_ref(drawable);
        let queue = device.ctx().shared().queue();
        queue.signalDrawable(drawable);
        drawable.present();
        Ok(false)
    }
}

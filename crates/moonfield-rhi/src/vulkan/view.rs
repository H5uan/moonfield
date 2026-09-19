//! Vulkan image-view wrapper.
//!
//! [`TextureView`] is the crate's vocabulary for an `vk::ImageView`: a
//! borrowed handle owned by a texture, offscreen target, or swapchain.

use crate::vulkan::device::DeviceContext;
use ash::vk;

/// A Vulkan image view wrapped for the RHI's resource vocabulary.
///
/// `owns` distinguishes a view created and owned by this wrapper (`from_raw`,
/// Drop destroys it) from one borrowed from another owner (`borrow_raw`, Drop
/// leaves it alone — the owner, e.g. `Texture` or `OffscreenTarget`,
/// destroys it).
pub struct TextureView {
    view: vk::ImageView,
    /// Keeps the device alive so an owned view can be destroyed in `Drop`.
    ctx: DeviceContext,
    owns: bool,
}

impl TextureView {
    /// Wrap an image view this wrapper owns; `Drop` destroys it.
    #[allow(dead_code)] // owned views are constructed by future owners
    pub(crate) fn from_raw(view: vk::ImageView, ctx: DeviceContext) -> Self {
        Self {
            view,
            ctx,
            owns: true,
        }
    }

    /// Borrow an image view owned elsewhere; `Drop` does not destroy it.
    pub(crate) fn borrow_raw(view: vk::ImageView, ctx: DeviceContext) -> Self {
        Self {
            view,
            ctx,
            owns: false,
        }
    }

    /// Raw Vulkan handle, for interop with libraries taking raw handles.
    pub(crate) fn raw_vk(&self) -> vk::ImageView {
        self.view
    }
}

impl Clone for TextureView {
    /// Clone shares the underlying image view: the clone never owns it, so
    /// only the original (if it owns) destroys the Vulkan object. This lets
    /// per-frame attachment records (`RenderAttachment`, view components)
    /// copy views without lifetime plumbing.
    fn clone(&self) -> Self {
        Self {
            view: self.view,
            ctx: self.ctx.clone(),
            owns: false,
        }
    }
}

impl Drop for TextureView {
    fn drop(&mut self) {
        if self.owns {
            // SAFETY: the device is valid (kept alive by `ctx`) and this
            // wrapper owns the view.
            unsafe {
                self.ctx.raw().destroy_image_view(self.view, None);
            }
        }
    }
}

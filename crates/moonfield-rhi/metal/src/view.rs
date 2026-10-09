//! Texture view wrapper.
//!
//! Metal has no separate image-view object; the view is the texture plus the
//! extent/format metadata the RHI's pass-recording vocabulary needs.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::MTLTexture;

use moonfield_rhi_core::Format;

/// A renderable/samplable view of a 2D texture.
#[derive(Clone)]
pub struct TextureView {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    width: u32,
    height: u32,
    format: Format,
}

impl TextureView {
    pub(crate) fn new(
        texture: Retained<ProtocolObject<dyn MTLTexture>>,
        width: u32,
        height: u32,
        format: Format,
    ) -> Self {
        Self {
            texture,
            width,
            height,
            format,
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The view's format.
    pub fn format(&self) -> Format {
        self.format
    }

    pub(crate) fn texture(&self) -> &ProtocolObject<dyn MTLTexture> {
        &self.texture
    }
}

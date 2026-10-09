//! GPU textures: a renderable 2D image with readback.

use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLDevice, MTLRegion, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureType,
    MTLTextureUsage,
};

use crate::device::Device;
use crate::formats::ToMetal;
use crate::view::TextureView;
use moonfield_rhi_core::{Error, Format, Result};

/// A 2D color target: render target + shader-readable, shared storage, with
/// CPU readback for tests and editor viewports.
#[derive(Clone)]
pub struct Texture {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    width: u32,
    height: u32,
    format: Format,
}

impl Texture {
    /// Create a `width`×`height` 2D render target of `format`.
    pub fn new_render_target(
        device: &Device,
        width: u32,
        height: u32,
        format: Format,
    ) -> Result<Self> {
        let shared = device.ctx().shared();
        let descriptor = MTLTextureDescriptor::new();
        descriptor.setTextureType(MTLTextureType::Type2D);
        descriptor.setPixelFormat(format.to_metal());
        // SAFETY: positive, small dimensions.
        unsafe {
            descriptor.setWidth(width as usize);
            descriptor.setHeight(height as usize);
        }
        descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        descriptor.setStorageMode(MTLStorageMode::Shared);
        let texture = shared
            .device()
            .newTextureWithDescriptor(&descriptor)
            .ok_or_else(|| Error::Backend("newTextureWithDescriptor failed".into()))?;
        shared.register_allocation(texture.as_ref());
        Ok(Self {
            texture,
            width,
            height,
            format,
        })
    }

    /// A view of the whole texture.
    pub fn view(&self) -> TextureView {
        TextureView::new(self.texture.clone(), self.width, self.height, self.format)
    }

    /// Read the whole texture back as tightly packed bytes (RGBA order for
    /// 4-byte formats).
    pub fn read_pixels(&self) -> Vec<u8> {
        let bytes = self.format.bytes_per_pixel();
        let mut out = vec![0u8; (self.width as usize) * (self.height as usize) * bytes];
        let region = MTLRegion {
            origin: objc2_metal::MTLOrigin { x: 0, y: 0, z: 0 },
            size: objc2_metal::MTLSize {
                width: self.width as usize,
                height: self.height as usize,
                depth: 1,
            },
        };
        // SAFETY: `out` holds width*height*bytes_per_row bytes, matching the
        // region; level 0, single-slice 2D texture.
        unsafe {
            let ptr = NonNull::new(out.as_mut_ptr().cast()).expect("readback buffer is non-empty");
            self.texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                ptr,
                self.width as usize * bytes,
                region,
                0,
            );
        }
        out
    }
}

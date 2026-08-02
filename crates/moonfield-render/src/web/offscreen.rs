//! Offscreen color target that can be sampled as a texture.
//!
//! Provides [`OffscreenTarget`], a renderable texture + view + sampler bundle
//! used for editor viewports: the scene is rendered into the texture and a UI
//! toolkit (e.g. egui-wgpu) samples it afterwards.

use crate::error::{Error, Result};
use crate::types::Format;
use crate::web::device::Device;

/// A renderable and sampleable offscreen color target.
pub struct OffscreenTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    format: Format,
    extent: (u32, u32),
}

impl OffscreenTarget {
    /// Create an offscreen target of `width`×`height` with the given color
    /// format. Zero dimensions are rejected.
    pub fn new(device: &Device, width: u32, height: u32, format: Format) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::Validation(format!(
                "offscreen target dimensions must be non-zero, got {}x{}",
                width, height
            )));
        }

        let texture = create_color_texture(device, width, height, format);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = create_sampler(device);

        Ok(Self {
            texture,
            view,
            sampler,
            format,
            extent: (width, height),
        })
    }

    /// Resize the target, recreating the texture and view.
    ///
    /// wgpu resources are refcounted and freed lazily, so no device idle wait
    /// is needed. Zero dimensions are ignored (e.g. a minimized viewport
    /// panel), mirroring the native backend.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        if self.extent == (width, height) {
            return Ok(());
        }

        self.texture = create_color_texture(device, width, height, self.format);
        self.view = self
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.extent = (width, height);
        Ok(())
    }

    /// Access the texture view (for sampling in a UI renderer, web only).
    pub fn texture_view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Access the sampler paired with the color texture (web only).
    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// The `(width, height)` of the target.
    pub fn extent(&self) -> (u32, u32) {
        self.extent
    }

    /// The color format of the target.
    pub fn format(&self) -> Format {
        self.format
    }
}

fn create_color_texture(device: &Device, width: u32, height: u32, format: Format) -> wgpu::Texture {
    device.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("moonfield-offscreen-target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: format.to_wgpu(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn create_sampler(device: &Device) -> wgpu::Sampler {
    device.device().create_sampler(&wgpu::SamplerDescriptor {
        label: Some("moonfield-offscreen-sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        ..Default::default()
    })
}

//! Vulkan pipeline layout abstraction.
//!
//! A [`PipelineLayout`] owns a `vk::PipelineLayout` and borrows the
//! [`BindGroupLayout`](crate::BindGroupLayout)s it was created from (those stay
//! caller-owned, mirroring how `GraphicsPipeline` borrows its `RenderPass`).
//! It is reusable across graphics and compute pipelines — the foundation for
//! descriptor-set binding in the RHI.
//!
//! Push-constant ranges are intentionally **not** exposed: the moonfield RHI
//! targets native (Vulkan) and web (wgpu) parity, and wgpu has no push
//! constants. Per-dispatch / per-draw parameters travel through a uniform
//! buffer binding instead (see the cross-phase decisions in
//! `physics-engine-foundation` / the `bind` module docs).

use crate::bind::BindGroupLayout;
use crate::error::{Error, Result};
use crate::native::device::Device;
use ash::vk;

/// A Vulkan pipeline layout owning its `vk::PipelineLayout` handle.
///
/// Borrows the [`BindGroupLayout`]s it references; the caller must keep them
/// alive for the lifetime of this layout (and any pipeline built from it).
pub struct PipelineLayout {
    layout: vk::PipelineLayout,
    device: ash::Device,
}

impl PipelineLayout {
    /// Create a pipeline layout from zero or more descriptor set layouts.
    ///
    /// The set layouts are bound in the order given: index `i` in `set_layouts`
    /// becomes set number `i` in the shader.
    pub fn new(device: &Device, set_layouts: &[&BindGroupLayout]) -> Result<Self> {
        let raw_layouts: Vec<vk::DescriptorSetLayout> =
            set_layouts.iter().map(|l| l.raw_vk()).collect();

        let create_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&raw_layouts);

        let layout = unsafe {
            device
                .raw()
                .create_pipeline_layout(&create_info, None)
                .map_err(|e| Error::Backend(format!("failed to create pipeline layout: {:?}", e)))?
        };

        Ok(Self {
            layout,
            device: device.raw().clone(),
        })
    }

    /// Create an empty pipeline layout (no descriptor sets).
    ///
    /// Used by the existing `GraphicsPipeline` path which historically built
    /// its own empty layout; new callers should pass an explicit slice.
    pub fn empty(device: &Device) -> Result<Self> {
        Self::new(device, &[])
    }

    /// Access the raw `vk::PipelineLayout` handle.
    pub fn raw(&self) -> vk::PipelineLayout {
        self.layout
    }
}

impl Drop for PipelineLayout {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline_layout(self.layout, None);
        }
    }
}

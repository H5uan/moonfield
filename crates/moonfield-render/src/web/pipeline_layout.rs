//! wgpu pipeline layout abstraction.
//!
//! Web counterpart of the native [`PipelineLayout`](crate::native::PipelineLayout).
//! Wraps a `wgpu::PipelineLayout` created from zero or more
//! [`BindGroupLayout`](crate::BindGroupLayout)s. Push constants are not exposed
//! on either backend (wgpu has none; native keeps parity); per-dispatch
//! parameters travel through a uniform buffer binding instead.

use crate::bind::BindGroupLayout;
use crate::error::Result;
use crate::web::device::Device;

/// A wgpu pipeline layout owning its `wgpu::PipelineLayout`.
pub struct PipelineLayout(wgpu::PipelineLayout);

impl PipelineLayout {
    /// Create a pipeline layout from zero or more descriptor set layouts.
    ///
    /// Set layouts bind in the order given: index `i` becomes set number `i`
    /// in the shader.
    pub fn new(device: &Device, set_layouts: &[&BindGroupLayout]) -> Result<Self> {
        let raw_layouts: Vec<Option<&wgpu::BindGroupLayout>> =
            set_layouts.iter().map(|l| Some(l.raw_wgpu())).collect();
        let layout = device
            .device()
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("moonfield-pipeline-layout"),
                bind_group_layouts: &raw_layouts,
                immediate_size: 0,
            });
        Ok(Self(layout))
    }

    /// Create an empty pipeline layout (no bind groups).
    pub fn empty(device: &Device) -> Result<Self> {
        Self::new(device, &[])
    }

    /// Access the raw `wgpu::PipelineLayout`.
    pub fn raw_wgpu(&self) -> &wgpu::PipelineLayout {
        &self.0
    }
}

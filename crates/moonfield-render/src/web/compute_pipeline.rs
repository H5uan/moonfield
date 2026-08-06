//! wgpu compute pipeline abstraction.
//!
//! Web counterpart of the native
//! [`ComputePipeline`](crate::native::ComputePipeline). Wraps a
//! `wgpu::ComputePipeline` built from a [`ShaderModule`] and a
//! [`PipelineLayout`]. Entry point is `"main"`, mirroring the native backend.
//!
//! Phase 0 guarantees this compiles on `wasm32-unknown-unknown`; behavior is
//! verified later (see the physics-engine-foundation plan).

use crate::error::Result;
use crate::web::device::Device;
use crate::web::pipeline_layout::PipelineLayout;
use crate::web::shader_module::ShaderModule;

/// A wgpu compute pipeline.
pub struct ComputePipeline(wgpu::ComputePipeline);

impl ComputePipeline {
    /// Create a compute pipeline from a shader module and layout.
    ///
    /// `layout` is borrowed (callers retain it). The shader entry point is
    /// `"main"`, matching the native backend's convention.
    pub fn new(
        device: &Device,
        layout: &PipelineLayout,
        compute_shader: &ShaderModule,
    ) -> Result<Self> {
        let pipeline = device
            .device()
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("moonfield-compute-pipeline"),
                layout: Some(layout.raw_wgpu()),
                module: compute_shader.raw(),
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        Ok(Self(pipeline))
    }

    /// Access the raw `wgpu::ComputePipeline`.
    pub fn raw_wgpu(&self) -> &wgpu::ComputePipeline {
        &self.0
    }
}

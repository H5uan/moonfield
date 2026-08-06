//! Vulkan compute pipeline abstraction.
//!
//! A [`ComputePipeline`] owns a `vk::Pipeline` and borrows the
//! [`PipelineLayout`](crate::native::PipelineLayout) and
//! [`ShaderModule`](crate::native::ShaderModule) it was created from (both stay
//! caller-owned). This mirrors the [`GraphicsPipeline`]↔`RenderPass` /
//! `ShaderModule` contract: the pipeline does not own its layout or shaders,
//! so callers retain them for the pipeline's lifetime.
//!
//! This is the compute counterpart to [`GraphicsPipeline`]: the same single-
//! stage, single-shader shape, just on `vk::PipelineBindPoint::COMPUTE`. It is
//! the entry point for GPU-driven physics (broadphase, narrowphase, solver,
//! integration) — see the physics-engine-foundation plan, Phase 0.

use crate::error::{Error, Result};
use crate::native::device::Device;
use crate::native::pipeline_layout::PipelineLayout;
use crate::native::shader_module::ShaderModule;
use ash::vk;

/// A Vulkan compute pipeline.
pub struct ComputePipeline {
    pipeline: vk::Pipeline,
    device: ash::Device,
}

impl ComputePipeline {
    /// Create a compute pipeline from a single SPIR-V shader module.
    ///
    /// `layout` is borrowed (must outlive the pipeline). The shader entry point
    /// is `main`.
    pub fn new(
        device: &Device,
        layout: &PipelineLayout,
        compute_shader: &ShaderModule,
    ) -> Result<Self> {
        let entry_name = std::ffi::CString::new("main").unwrap();
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(compute_shader.raw())
            .name(&entry_name);

        let create_info =
            vk::ComputePipelineCreateInfo::default().stage(stage).layout(layout.raw());

        // `create_compute_pipelines` returns a `Result<Vec<vk::Pipeline>,
        // vk::Result>` (with a pipeline cache); a single-element slice yields
        // one pipeline on success.
        let pipelines = unsafe {
            device.raw().create_compute_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&create_info),
                None,
            )
        }
        .map_err(|e| Error::Backend(format!("failed to create compute pipeline: {:?}", e)))?;

        Ok(Self {
            pipeline: pipelines[0],
            device: device.raw().clone(),
        })
    }

    /// Access the raw `vk::Pipeline` handle.
    pub fn raw(&self) -> vk::Pipeline {
        self.pipeline
    }
}

impl Drop for ComputePipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_pipeline(self.pipeline, None);
        }
    }
}

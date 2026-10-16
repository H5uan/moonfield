//! Optimizer kernels.
//!
//! Adam is the one sanctioned optimizer. It runs as a compute kernel
//! (`assets/shaders/ml/adam.slang`) over flat `f32` parameter buffers: one
//! thread per scalar, moments kept in GPU-resident buffers owned here.

use moonfield_rhi::{
    CommandBuffer, ComputePipeline, Device, GpuAllocation, MemoryResidency, Result, RootBinder,
    RootParamPlace, ShaderModule,
};
use moonfield_shader::Shader;

/// Byte-identical mirror of `AdamConfig` in `assets/shaders/ml/adam.slang`
/// (natural layout: six 4-byte scalars, no padding).
#[repr(C)]
#[derive(Clone, Copy)]
struct AdamConfig {
    step: u32,
    count: u32,
    lr: f32,
    beta1: f32,
    beta2: f32,
    epsilon: f32,
}

/// Hyperparameters for the Adam update kernel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdamParams {
    /// Learning rate.
    pub lr: f32,
    /// First-moment decay.
    pub beta1: f32,
    /// Second-moment decay.
    pub beta2: f32,
    /// Numerical epsilon.
    pub epsilon: f32,
}

impl Default for AdamParams {
    fn default() -> Self {
        Self {
            lr: 1e-2,
            beta1: 0.9,
            beta2: 0.999,
            epsilon: 1e-8,
        }
    }
}

/// The Adam kernel's five root-pointer placements, resolved once at pipeline
/// build (see `RootBinder::pointer_param`).
struct AdamPlaces {
    params: RootParamPlace,
    grads: RootParamPlace,
    moment1: RootParamPlace,
    moment2: RootParamPlace,
    config: RootParamPlace,
}

/// GPU-side Adam state for one flat `f32` parameter buffer.
///
/// Owns the first/second moment buffers (same element count as the parameter
/// buffer) and the compute pipeline built from the Adam kernel.
pub struct Adam {
    params: AdamParams,
    param_count: u32,
    moment1: GpuAllocation,
    moment2: GpuAllocation,
    config: GpuAllocation,
    pipeline: ComputePipeline,
    places: AdamPlaces,
}

impl Adam {
    /// Allocates moment buffers matching `param_count` scalars and builds the
    /// Adam compute pipeline on `device` from the `shader` asset's source
    /// (memoized by the device shader cache).
    pub fn new(
        device: &Device,
        shader: &Shader,
        param_count: usize,
        params: AdamParams,
    ) -> Result<Self> {
        let cache = device.shader_cache();
        let reflection =
            cache.compile_source_reflection(shader.path(), shader.source(), &["adam_step"])?;

        let binder = RootBinder::new(&reflection, "adam_step")?;
        binder.validate_parameters(&[
            ("params", moonfield_rhi::RootParamKind::Pointer),
            ("grads", moonfield_rhi::RootParamKind::Pointer),
            ("moment1", moonfield_rhi::RootParamKind::Pointer),
            ("moment2", moonfield_rhi::RootParamKind::Pointer),
            ("config", moonfield_rhi::RootParamKind::Pointer),
        ])?;
        validate_adam_abi(&reflection)?;
        let places = AdamPlaces {
            params: binder.pointer_param("params")?,
            grads: binder.pointer_param("grads")?,
            moment1: binder.pointer_param("moment1")?,
            moment2: binder.pointer_param("moment2")?,
            config: binder.pointer_param("config")?,
        };
        let compiled =
            cache.compile_source(shader.path(), shader.source(), "adam_step", &[], &[])?;
        let module = ShaderModule::from_compiled(device, &compiled)?;
        let pipeline = ComputePipeline::new(device, &module)?;
        let bytes = (param_count * size_of::<f32>()) as u64;
        let moment1 = GpuAllocation::new(device, bytes, MemoryResidency::CpuWriteOnly)?;
        let moment2 = GpuAllocation::new(device, bytes, MemoryResidency::CpuWriteOnly)?;
        let config = GpuAllocation::new(
            device,
            size_of::<AdamConfig>() as u64,
            MemoryResidency::CpuWriteOnly,
        )?;

        // SAFETY: both moment allocations are host-visible, persistently
        // mapped, and sized `param_count` floats; the moving averages must
        // start at zero.
        for alloc in [&moment1, &moment2] {
            unsafe {
                std::ptr::write_bytes(
                    alloc
                        .host()
                        .expect("moments must be host-visible")
                        .typed::<f32>(),
                    0,
                    param_count,
                );
            }
        }
        // SAFETY: the config allocation is host-visible, persistently mapped,
        // and sized one `AdamConfig`.
        unsafe {
            *config
                .host()
                .expect("config must be host-visible")
                .typed::<AdamConfig>() = AdamConfig {
                step: 0,
                count: param_count as u32,
                lr: params.lr,
                beta1: params.beta1,
                beta2: params.beta2,
                epsilon: params.epsilon,
            };
        }
        Ok(Self {
            params,
            param_count: param_count as u32,
            moment1,
            moment2,
            config,
            pipeline,
            places,
        })
    }

    /// Hyperparameters this optimizer was created with.
    pub fn params(&self) -> AdamParams {
        self.params
    }

    /// Appends the Adam update dispatch for `params_buf` given `grads`.
    ///
    /// Only this optimizer's own dispatch is recorded — the caller orders it
    /// against the gradient-producing dispatches with its own barriers.
    pub fn record_step(
        &self,
        cmd: &CommandBuffer,
        params_buf: &GpuAllocation,
        grads: &GpuAllocation,
        step: u32,
    ) {
        // SAFETY: the config allocation is host-visible, persistently mapped,
        // and sized one `AdamConfig` whose first field is `step`; the
        // training loop is synchronous (`submit_and_wait` between steps), so
        // no in-flight dispatch reads it.
        unsafe {
            *self
                .config
                .host()
                .expect("config must be host-visible")
                .typed::<u32>() = step;
        }

        cmd.bind_compute_pipeline(&self.pipeline);
        push_ptr(cmd, &self.places.params, params_buf);
        push_ptr(cmd, &self.places.grads, grads);
        push_ptr(cmd, &self.places.moment1, &self.moment1);
        push_ptr(cmd, &self.places.moment2, &self.moment2);
        push_ptr(cmd, &self.places.config, &self.config);
        cmd.dispatch(self.param_count.div_ceil(256), 1, 1);
    }
}

fn validate_adam_abi(reflection: &moonfield_rhi::Reflection) -> Result<()> {
    reflection
        .entry_parameter_pointee_layout("adam_step", "config")?
        .validate_rust_layout::<AdamConfig>(&[
            ("step", std::mem::offset_of!(AdamConfig, step)),
            ("count", std::mem::offset_of!(AdamConfig, count)),
            ("lr", std::mem::offset_of!(AdamConfig, lr)),
            ("beta1", std::mem::offset_of!(AdamConfig, beta1)),
            ("beta2", std::mem::offset_of!(AdamConfig, beta2)),
            ("epsilon", std::mem::offset_of!(AdamConfig, epsilon)),
        ])?;
    for parameter in ["params", "grads", "moment1", "moment2"] {
        reflection
            .entry_parameter_pointee_layout("adam_step", parameter)?
            .validate_rust_layout::<f32>(&[])?;
    }
    Ok(())
}

/// Push one pointer root parameter at its reflected place.
fn push_ptr(cmd: &CommandBuffer, place: &RootParamPlace, alloc: &GpuAllocation) {
    let bytes = place
        .pointer_bytes(alloc.gpu_ptr().as_raw())
        .expect("adam root pointer placement");
    cmd.push_data(place.offset as u32, &bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adam_runtime_abi_accepts_defaults_and_rejects_reordered_config() {
        let path = moonfield_asset::assets_dir().join("shaders/ml/adam.slang");
        let source = std::fs::read_to_string(path).unwrap();
        let reordered = source.replace(
            "uint32_t step;\n    uint32_t count;",
            "uint32_t count;\n    uint32_t step;",
        );
        assert_ne!(source, reordered);
        let compiler = moonfield_rhi::Compiler::new().unwrap();
        for (source, valid) in [(source, true), (reordered, false)] {
            let reflection = compiler
                .compile_source_to_reflection(
                    "adam_abi",
                    &source,
                    &["adam_step"],
                    moonfield_rhi::ShaderTarget::Spirv,
                )
                .unwrap();
            assert_eq!(validate_adam_abi(&reflection).is_ok(), valid);
        }
    }

    /// Guards the `#[repr(C)]` mirror against field-type changes: the Slang
    /// side expects six 4-byte scalars.
    #[test]
    fn adam_config_is_24_bytes() {
        assert_eq!(size_of::<AdamConfig>(), 24);
    }
}

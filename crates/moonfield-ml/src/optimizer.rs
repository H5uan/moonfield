//! Optimizer kernels.
//!
//! Adam is the one sanctioned optimizer. It runs as a compute kernel
//! (`assets/shaders/ml/adam.slang`) over flat `f32` parameter buffers: one
//! thread per scalar, moments kept in GPU-resident buffers owned here.

use moonfield_rhi::{CommandBuffer, Device, GpuAllocation};

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

/// GPU-side Adam state for one flat `f32` parameter buffer.
///
/// Owns the first/second moment buffers (same element count as the parameter
/// buffer) and the compute pipeline built from the Adam kernel.
pub struct Adam {
    params: AdamParams,
    // TODO: moment1/moment2 GpuAllocations, ComputePipeline, meta buffer
    // holding the iteration counter for bias correction.
}

impl Adam {
    /// Allocates moment buffers matching `param_count` scalars and builds the
    /// Adam compute pipeline on `device`.
    pub fn new(device: &Device, param_count: usize, params: AdamParams) -> Self {
        let _ = (device, param_count);
        Self { params }
    }

    /// Hyperparameters this optimizer was created with.
    pub fn params(&self) -> AdamParams {
        self.params
    }

    /// Appends the Adam update dispatch for `params_buf` given `grads`.
    pub fn record_step(
        &self,
        cmd: &CommandBuffer,
        params_buf: &GpuAllocation,
        grads: &GpuAllocation,
        step: u32,
    ) {
        let _ = (cmd, params_buf, grads, step);
        todo!("bind pipeline, push root pointers, dispatch param_count/256")
    }
}

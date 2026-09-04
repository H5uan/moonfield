//! The host-side training loop.
//!
//! A [`TrainingMethod`] records one optimization step into a command buffer
//! (forward, backward, gradient reduction, optimizer update, with barriers
//! between stages — the `gaussian_fit` RHI test is the reference shape).
//! [`Trainer`] owns the submission cadence and progress reporting; methods
//! stay agnostic of both.

use moonfield_rhi::{CommandBuffer, Device};

/// One trainable method (e.g. Gaussian Splatting).
pub trait TrainingMethod {
    /// Records one full optimization step into `cmd`.
    ///
    /// The implementation appends its kernel dispatches and the barriers
    /// between them; `step` is the 1-based iteration index (optimizer bias
    /// correction depends on it).
    fn record_step(&mut self, cmd: &CommandBuffer, step: u32);

    /// Reads the current scalar loss back to the host for progress reporting.
    fn readback_loss(&mut self) -> f32;
}

/// Drives a [`TrainingMethod`] for a fixed number of steps on one device.
pub struct Trainer {
    // TODO: command pool, loss-reporting cadence, checkpoint hooks.
}

impl Trainer {
    /// Creates a trainer submitting on `device`'s graphics queue.
    pub fn new(device: &Device) -> Self {
        let _ = device;
        todo!("command pool + submission state")
    }

    /// Runs `method` for `steps` iterations: record, submit, wait; read back
    /// and log the loss on the reporting cadence.
    pub fn run<M: TrainingMethod>(&mut self, method: &mut M, steps: u32) {
        let _ = method;
        let _ = steps;
        todo!("record/submit loop with loss reporting")
    }
}

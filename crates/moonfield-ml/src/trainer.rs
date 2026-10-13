//! The host-side training loop.
//!
//! A [`TrainingMethod`] records one optimization step into a command buffer
//! (forward, backward, gradient reduction, optimizer update, with barriers
//! between stages — the `gaussian_fit` RHI test is the reference shape).
//! [`Trainer`] owns the submission cadence and progress reporting; methods
//! stay agnostic of both.

use moonfield_rhi::{CommandBuffer, CommandBufferUsage, Device, Result};

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
pub struct Trainer<'a> {
    device: &'a Device,
    report_every: u32,
}

impl<'a> Trainer<'a> {
    /// Creates a trainer submitting on `device`'s graphics queue.
    pub fn new(device: &'a Device, report_every: u32) -> Result<Self> {
        Ok(Self {
            device,
            report_every,
        })
    }

    /// Runs `method` for `steps` iterations: take a fresh command buffer from
    /// the device, record the step, submit and wait; read back and log the
    /// loss on the reporting cadence.
    pub fn run<M: TrainingMethod>(&mut self, method: &mut M, steps: u32) {
        let every = self.report_every.max(1);
        for step in 1..=steps {
            let cmd = self
                .device
                .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
                .expect("command buffer");
            method.record_step(&cmd, step);
            self.device
                .submit_and_wait(&[&cmd])
                .expect("submit and wait");
            if step == 1 || step % every == 0 || step == steps {
                let loss = method.readback_loss();
                tracing::info!(step, loss, "training step");
            }
        }
    }
}

//! The host-side training loop.
//!
//! A [`TrainingMethod`] records one optimization step into a command buffer
//! (forward, backward, gradient reduction, optimizer update, with barriers
//! between stages — the `gaussian_fit` RHI test is the reference shape).
//! [`Trainer`] owns the submission cadence and progress reporting; methods
//! stay agnostic of both.

use moonfield_rhi::{CommandBuffer, CommandBufferUsage, CommandPool, Device, Result};

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
    /// Drops before the pool: Rust drops fields in declaration order, and
    /// the buffer frees itself through the pool handle in `Drop`.
    cmd: CommandBuffer,
    /// Held for drop order only: the pool must outlive its command buffer.
    #[allow(dead_code)]
    pool: CommandPool,
}

impl<'a> Trainer<'a> {
    /// Creates a trainer submitting on `device`'s graphics queue.
    pub fn new(device: &'a Device, report_every: u32) -> Result<Self> {
        let pool = CommandPool::new(device, device.queue_family_indices().graphics)?;
        let cmd = pool.allocate_command_buffer()?;
        Ok(Self {
            device,
            pool,
            report_every,
            cmd,
        })
    }

    /// Runs `method` for `steps` iterations: record, submit, wait; read back
    /// and log the loss on the reporting cadence.
    pub fn run<M: TrainingMethod>(&mut self, method: &mut M, steps: u32) {
        let every = self.report_every.max(1);
        for step in 1..=steps {
            self.cmd
                .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
                .expect("begin");
            method.record_step(&self.cmd, step);
            self.cmd.end().expect("end");
            self.device
                .submit_and_wait(&[&self.cmd])
                .expect("submit and wait");
            if step == 1 || step % every == 0 || step == steps {
                let loss = method.readback_loss();
                tracing::info!(step, loss, "training step");
            }
        }
    }
}

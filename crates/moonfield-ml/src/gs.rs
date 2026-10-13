//! The Gaussian Splatting training method.
//!
//! 3DGS, 2DGS, and the Stoch3DGS stochastic ray-tracing variant share this
//! scaffolding: the same trainable parameter set and optimizer, different
//! forward/backward kernel families (rasterization vs. sorting-free
//! stochastic ray tracing). Domain types (`GaussianScene`, `SplatCloud`,
//! COLMAP / glTF I/O) live in `moonfield_render_feature::splat`; this module
//! owns only training state.

use moonfield_render_feature::splat::scene::GaussianScene;
use moonfield_rhi::CommandBuffer;

use crate::trainer::TrainingMethod;

/// A [`GaussianScene`] plus its training state.
///
/// Owns the per-attribute GPU mirrors of the scene's SoA parameter arrays,
/// their gradient buffers, and the densification bookkeeping (clone / split /
/// prune). Attribute layout follows `GaussianScene`'s SoA contract so export
/// back into a `SplatCloud` is a plain download.
pub struct TrainableScene {
    /// Canonical host copy of the trainable parameters.
    scene: GaussianScene,
    // TODO: per-attribute param/gradient GpuAllocations, Adam state,
    // densification accumulators.
}

impl TrainableScene {
    /// Uploads `scene` onto the training device and allocates the training
    /// state for it.
    pub fn new(scene: GaussianScene) -> Self {
        Self { scene }
    }

    /// The canonical host copy of the trainable parameters.
    pub fn scene(&self) -> &GaussianScene {
        &self.scene
    }
}

impl TrainingMethod for TrainableScene {
    fn record_step(&mut self, cmd: &CommandBuffer, step: u32) {
        let _ = (cmd, step);
        todo!("forward, backward, reduce, adam dispatches with barriers")
    }

    fn readback_loss(&mut self) -> f32 {
        todo!("loss buffer download")
    }
}

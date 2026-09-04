//! Parameter checkpoints.
//!
//! A checkpoint snapshots a method's trainable parameters mid-training and
//! restores them later. Serialization format is method-defined; Gaussian
//! Splatting checkpoints double as exportable `KHR_gaussian_splatting` glTF
//! through `moonfield_render_feature::splat::io`.

use std::path::Path;

/// Save/restore for a method's trainable parameters.
pub trait Checkpoint: Sized {
    /// The save/restore error.
    type Error;

    /// Writes the current parameters to `path`.
    fn save(&self, path: &Path) -> Result<(), Self::Error>;

    /// Loads parameters from `path`.
    fn load(path: &Path) -> Result<Self, Self::Error>;
}

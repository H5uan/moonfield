//! The `SplatCloud` asset: a loaded 3D Gaussian scene plus load metadata.
//!
//! Plain data with no ECS dependency — `moonfield-renderer` stays ECS-free.
//! The ECS glue is the [`SplatCloudHandle`] component wrapper (a component
//! through the blanket `Component` impl), and the store is the
//! `Assets<SplatCloud>` world resource; the caller loads files synchronously
//! (see [`SplatCloud::from_ply_file`]) and inserts them.
//!
//! Training/optimizer state deliberately stays outside the `World` (see
//! [`crate::splat::train`]) — the asset is the immutable cloud a scene entity
//! references.

use std::path::Path;

use moonfield_asset::Handle;

use crate::splat::io::ply::{parse_ply, PlyError};
use crate::splat::scene::GaussianScene;

/// Errors loading a [`SplatCloud`] from a file.
#[derive(Debug, thiserror::Error)]
pub enum SplatLoadError {
    /// The file could not be read.
    #[error("failed to read `{path}`: {source}")]
    Io {
        /// The I/O error.
        source: std::io::Error,
        /// The file that failed.
        path: std::path::PathBuf,
    },
    /// The file is not a valid 3DGS PLY.
    #[error("failed to parse `{path}`: {source}")]
    Ply {
        /// The parse error.
        source: PlyError,
        /// The file that failed.
        path: std::path::PathBuf,
    },
}

/// A loaded 3D Gaussian splat cloud asset.
///
/// Wraps the CPU-side [`GaussianScene`] SoA data (uploaded to the GPU by the
/// render path when one exists) plus the source path for editor display and
/// a precomputed axis-aligned bounding box.
pub struct SplatCloud {
    scene: GaussianScene,
    /// What the cloud was loaded from (e.g. the PLY path), for display.
    source: Option<String>,
    aabb_min: [f32; 3],
    aabb_max: [f32; 3],
}

impl SplatCloud {
    /// Wrap an already-parsed scene, computing its AABB (empty scenes get a
    /// unit box at the origin).
    pub fn new(scene: GaussianScene, source: Option<String>) -> Self {
        let (aabb_min, aabb_max) = compute_aabb(&scene);
        Self {
            scene,
            source,
            aabb_min,
            aabb_max,
        }
    }

    /// Parse a 3DGS PLY from memory.
    pub fn from_ply_bytes(bytes: &[u8], source: Option<String>) -> Result<Self, PlyError> {
        parse_ply(bytes).map(|scene| Self::new(scene, source))
    }

    /// Load and parse a 3DGS PLY file (synchronous).
    pub fn from_ply_file(path: impl AsRef<Path>) -> Result<Self, SplatLoadError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| SplatLoadError::Io {
            source: e,
            path: path.to_path_buf(),
        })?;
        let source = Some(path.display().to_string());
        Self::from_ply_bytes(&bytes, source).map_err(|e| SplatLoadError::Ply {
            source: e,
            path: path.to_path_buf(),
        })
    }

    /// The underlying Gaussian scene.
    pub fn scene(&self) -> &GaussianScene {
        &self.scene
    }

    /// Number of Gaussians in the cloud.
    pub fn len(&self) -> usize {
        self.scene.len()
    }

    /// Whether the cloud contains no Gaussians.
    pub fn is_empty(&self) -> bool {
        self.scene.is_empty()
    }

    /// What the cloud was loaded from, if known.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Axis-aligned bounding box of the Gaussian centers, `(min, max)`.
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        (self.aabb_min, self.aabb_max)
    }
}

fn compute_aabb(scene: &GaussianScene) -> ([f32; 3], [f32; 3]) {
    if scene.is_empty() {
        return ([-0.5; 3], [0.5; 3]);
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in &scene.positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    (min, max)
}

/// ECS component: the entity renders the referenced splat cloud.
///
/// A plain newtype — a component through the blanket `Component` impl, so
/// this crate does not depend on the ECS. Store the assets in the
/// `Assets<SplatCloud>` world resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplatCloudHandle(pub Handle<SplatCloud>);

impl SplatCloudHandle {
    /// The underlying asset handle.
    pub fn handle(&self) -> Handle<SplatCloud> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_scene() -> GaussianScene {
        GaussianScene {
            positions: vec![[0.0, 0.0, 0.0], [2.0, -1.0, 4.0]],
            scales: vec![[0.0; 3]; 2],
            rotations: vec![[1.0, 0.0, 0.0, 0.0]; 2],
            opacities: vec![0.0; 2],
            sh_dc: vec![[0.0; 3]; 2],
            sh_rest: vec![[0.0; 45]; 2],
        }
    }

    #[test]
    fn test_splat_cloud_aabb_and_accessors() {
        let cloud = SplatCloud::new(sample_scene(), Some("test.ply".into()));
        assert_eq!(cloud.len(), 2);
        assert_eq!(cloud.source(), Some("test.ply"));
        assert_eq!(cloud.aabb(), ([0.0, -1.0, 0.0], [2.0, 0.0, 4.0]));
        assert_eq!(cloud.scene().positions.len(), 2);
    }

    #[test]
    fn test_empty_cloud_gets_unit_aabb() {
        let cloud = SplatCloud::new(GaussianScene::default(), None);
        assert!(cloud.is_empty());
        assert_eq!(cloud.aabb(), ([-0.5; 3], [0.5; 3]));
    }

    #[test]
    fn test_from_ply_bytes_invalid_is_rejected() {
        assert!(SplatCloud::from_ply_bytes(b"not a ply", None).is_err());
    }
}

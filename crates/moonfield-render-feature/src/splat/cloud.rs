//! The `SplatCloud` asset: a loaded 3D Gaussian scene plus load metadata.
//!
//! [`SplatCloud`] is plain source data. The same module owns the
//! [`SplatCloudHandle`] ECS component. The caller loads files synchronously
//! (see [`SplatCloud::from_gltf_file`]) and inserts them into `Assets<SplatCloud>`.
//!
//! Training/optimizer state deliberately stays outside the `World` (see
//! [`crate::splat::train`]) — the asset is the immutable cloud a scene entity
//! references.

use std::path::Path;

use moonfield_asset::Handle;

use crate::splat::io::gltf::{load_splat_gltf, SplatGltfError};
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
    /// The file is not a valid `KHR_gaussian_splatting` glTF.
    #[error("failed to parse `{path}`: {source}")]
    Gltf {
        /// The parse error.
        source: SplatGltfError,
        /// The file that failed.
        path: std::path::PathBuf,
    },
}

/// A loaded 3D Gaussian splat cloud asset.
///
/// Wraps the CPU-side [`GaussianScene`] SoA data (uploaded to the GPU by the
/// render path when one exists) plus the source path for editor display and
/// a precomputed axis-aligned bounding box.
#[derive(Clone)]
pub struct SplatCloud {
    scene: GaussianScene,
    /// What the cloud was loaded from (e.g. the glTF path), for display.
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

    /// Parse a `KHR_gaussian_splatting` glTF/GLB from memory. External buffer
    /// references are not resolvable here — use [`SplatCloud::from_gltf_file`]
    /// for files that have them.
    pub fn from_gltf_bytes(bytes: &[u8], source: Option<String>) -> Result<Self, SplatGltfError> {
        load_splat_gltf(bytes, None).map(|scene| Self::new(scene, source))
    }

    /// Load and parse a `KHR_gaussian_splatting` glTF/GLB file (synchronous).
    pub fn from_gltf_file(path: impl AsRef<Path>) -> Result<Self, SplatLoadError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| SplatLoadError::Io {
            source: e,
            path: path.to_path_buf(),
        })?;
        let source = Some(path.display().to_string());
        // External buffers resolve relative to the file's directory.
        load_splat_gltf(&bytes, path.parent())
            .map(|scene| Self::new(scene, source))
            .map_err(|e| SplatLoadError::Gltf {
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
/// A plain newtype — a component through `moonfield-ecs`'s blanket
/// `Component` impl. Store the assets in the `Assets<SplatCloud>` world
/// resource.
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

    /// A minimal one-splat GLB (degree-0 SH only) at position [1, 2, 3].
    fn one_splat_glb() -> Vec<u8> {
        let f32_bytes = |values: &[f32]| {
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        let mut bin = f32_bytes(&[1.0, 2.0, 3.0]);
        bin.extend_from_slice(&f32_bytes(&[0.0, 0.0, 0.0, 1.0]));
        bin.extend_from_slice(&f32_bytes(&[1.0, 1.0, 1.0]));
        bin.extend_from_slice(&f32_bytes(&[0.5]));
        bin.extend_from_slice(&f32_bytes(&[0.1, 0.2, 0.3]));

        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 12}},
                    {{"buffer": 0, "byteOffset": 12, "byteLength": 16}},
                    {{"buffer": 0, "byteOffset": 28, "byteLength": 12}},
                    {{"buffer": 0, "byteOffset": 40, "byteLength": 4}},
                    {{"buffer": 0, "byteOffset": 44, "byteLength": 12}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
                      "min": [1.0, 2.0, 3.0], "max": [1.0, 2.0, 3.0]}},
                    {{"bufferView": 1, "componentType": 5126, "count": 1, "type": "VEC4"}},
                    {{"bufferView": 2, "componentType": 5126, "count": 1, "type": "VEC3"}},
                    {{"bufferView": 3, "componentType": 5126, "count": 1, "type": "SCALAR"}},
                    {{"bufferView": 4, "componentType": 5126, "count": 1, "type": "VEC3"}}
                ],
                "meshes": [{{"primitives": [{{
                    "attributes": {{
                        "POSITION": 0,
                        "KHR_gaussian_splatting:ROTATION": 1,
                        "KHR_gaussian_splatting:SCALE": 2,
                        "KHR_gaussian_splatting:OPACITY": 3,
                        "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0": 4
                    }},
                    "mode": 0,
                    "extensions": {{"KHR_gaussian_splatting": {{"kernel": "ellipse", "colorSpace": "srgb_rec709_display"}}}}
                }}]}}]
            }}"#,
            bin.len()
        );
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    #[test]
    fn test_splat_cloud_aabb_and_accessors() {
        let cloud = SplatCloud::new(sample_scene(), Some("test.gltf".into()));
        assert_eq!(cloud.len(), 2);
        assert_eq!(cloud.source(), Some("test.gltf"));
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
    fn test_from_gltf_bytes_loads_cloud() {
        let cloud = SplatCloud::from_gltf_bytes(&one_splat_glb(), Some("one.glb".into())).unwrap();
        assert_eq!(cloud.len(), 1);
        assert_eq!(cloud.source(), Some("one.glb"));
        assert_eq!(cloud.aabb(), ([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]));
    }

    #[test]
    fn test_from_gltf_bytes_invalid_is_rejected() {
        assert!(SplatCloud::from_gltf_bytes(b"not a gltf", None).is_err());
    }
}

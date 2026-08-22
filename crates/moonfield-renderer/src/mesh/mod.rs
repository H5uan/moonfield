//! The `Mesh` asset: merged triangle geometry loaded from a glTF file.
//!
//! Plain data with no ECS dependency — `moonfield-renderer` stays ECS-free.
//! The ECS glue is the [`MeshHandle`] component wrapper (a component through
//! the blanket `Component` impl), and the store is the `Assets<Mesh>` world
//! resource; the caller loads files synchronously (see
//! [`Mesh::from_gltf_file`]) and inserts them. [`MeshRenderer`] is the
//! renderable component pairing a mesh with a flat color.

pub mod gltf;

use std::path::Path;

use moonfield_asset::Handle;

use crate::mesh::gltf::{import_gltf_mesh, parse_gltf_mesh, MeshGltfError};

/// Errors loading a [`Mesh`] from a file.
#[derive(Debug, thiserror::Error)]
pub enum MeshLoadError {
    /// The file could not be read.
    #[error("failed to read `{path}`: {source}")]
    Io {
        /// The I/O error.
        source: std::io::Error,
        /// The file that failed.
        path: std::path::PathBuf,
    },
    /// The file is not a valid triangle-mesh glTF.
    #[error("failed to parse `{path}`: {source}")]
    Gltf {
        /// The parse error.
        source: MeshGltfError,
        /// The file that failed.
        path: std::path::PathBuf,
    },
}

/// A loaded triangle-mesh asset.
///
/// All triangle primitives of the source glTF are merged into a single
/// positions + indices pair (node hierarchy and materials are dropped; see
/// [`crate::mesh::gltf`]). Carries the source path for editor display and a
/// precomputed axis-aligned bounding box.
pub struct Mesh {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    /// What the mesh was loaded from (e.g. the glTF path), for display.
    source: Option<String>,
    aabb_min: [f32; 3],
    aabb_max: [f32; 3],
}

impl Mesh {
    /// Wrap merged geometry, computing its AABB (empty meshes get a unit box
    /// at the origin).
    pub fn new(positions: Vec<[f32; 3]>, indices: Vec<u32>, source: Option<String>) -> Self {
        let (aabb_min, aabb_max) = compute_aabb(&positions);
        Self {
            positions,
            indices,
            source,
            aabb_min,
            aabb_max,
        }
    }

    /// Import a triangle mesh from in-memory glTF/GLB bytes.
    pub fn from_gltf_bytes(bytes: &[u8], source: Option<String>) -> Result<Self, MeshGltfError> {
        import_gltf_mesh(bytes).map(|(positions, indices)| Self::new(positions, indices, source))
    }

    /// Load and import a triangle mesh from a glTF/GLB file (synchronous).
    pub fn from_gltf_file(path: impl AsRef<Path>) -> Result<Self, MeshLoadError> {
        let path = path.as_ref();
        // `gltf::import` resolves external buffers relative to the file.
        let (document, buffers, _images) = ::gltf::import(path).map_err(|e| match e {
            ::gltf::Error::Io(source) => MeshLoadError::Io {
                source,
                path: path.to_path_buf(),
            },
            source => MeshLoadError::Gltf {
                source: MeshGltfError::from(source),
                path: path.to_path_buf(),
            },
        })?;
        let source = Some(path.display().to_string());
        parse_gltf_mesh(&document, &buffers)
            .map(|(positions, indices)| Self::new(positions, indices, source))
            .map_err(|e| MeshLoadError::Gltf {
                source: e,
                path: path.to_path_buf(),
            })
    }

    /// Vertex positions, three per triangle corner.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Triangle indices into [`Mesh::positions`].
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// What the mesh was loaded from, if known.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Axis-aligned bounding box of the vertices, `(min, max)`.
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        (self.aabb_min, self.aabb_max)
    }
}

fn compute_aabb(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    if positions.is_empty() {
        return ([-0.5; 3], [0.5; 3]);
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in positions {
        for axis in 0..3 {
            min[axis] = min[axis].min(p[axis]);
            max[axis] = max[axis].max(p[axis]);
        }
    }
    (min, max)
}

/// ECS component: the entity renders the referenced mesh.
///
/// A plain newtype — a component through the blanket `Component` impl, so
/// this crate does not depend on the ECS. Store the assets in the
/// `Assets<Mesh>` world resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshHandle(pub Handle<Mesh>);

impl MeshHandle {
    /// The underlying asset handle.
    pub fn handle(&self) -> Handle<Mesh> {
        self.0
    }
}

/// ECS component: renders the referenced mesh with a flat color.
///
/// `mesh` is excluded from reflection (`#[reflect(ignore)]`) — handles are
/// not field-editable; the Inspector edits `color`.
#[derive(Debug, Clone, Copy, PartialEq, moonfield_reflect::Reflect)]
pub struct MeshRenderer {
    /// The mesh to render.
    #[reflect(ignore)]
    pub mesh: MeshHandle,
    /// Flat color, linear RGBA.
    pub color: [f32; 4],
}

impl MeshRenderer {
    /// A mesh rendered with the given color.
    pub fn new(mesh: MeshHandle, color: [f32; 4]) -> Self {
        Self { mesh, color }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mesh_new_computes_aabb() {
        let mesh = Mesh::new(
            vec![[0.0, 0.0, 0.0], [2.0, -1.0, 4.0]],
            vec![0, 1, 1],
            Some("test.gltf".into()),
        );
        assert_eq!(mesh.positions().len(), 2);
        assert_eq!(mesh.indices(), &[0, 1, 1]);
        assert_eq!(mesh.source(), Some("test.gltf"));
        assert_eq!(mesh.aabb(), ([0.0, -1.0, 0.0], [2.0, 0.0, 4.0]));
    }

    #[test]
    fn test_empty_mesh_gets_unit_aabb() {
        let mesh = Mesh::new(Vec::new(), Vec::new(), None);
        assert_eq!(mesh.aabb(), ([-0.5; 3], [0.5; 3]));
    }

    #[test]
    fn test_mesh_renderer_reflects_color_only() {
        use moonfield_reflect::Reflect;
        let mut assets = moonfield_asset::Assets::<Mesh>::default();
        let handle = MeshHandle(assets.add(Mesh::new(Vec::new(), Vec::new(), None)));
        let renderer = MeshRenderer::new(handle, [1.0, 0.5, 0.25, 1.0]);
        let fields = renderer.field_infos();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "color");
        assert!(renderer.field("color").is_some());
        assert!(renderer.field("mesh").is_none());
    }
}

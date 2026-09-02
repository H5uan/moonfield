//! The `Mesh` asset: merged triangle geometry loaded from a glTF file.
//!
//! [`Mesh`] is plain source data. The same module owns the small ECS/render
//! adapter: [`MeshHandle`], [`MeshRenderer`], render-world extraction, and the
//! revision-keyed prepared cache. The caller loads files synchronously (see
//! [`Mesh::from_gltf_file`]) and inserts them into `Assets<Mesh>`.

pub mod gltf;

use std::path::Path;
use std::{collections::HashMap, collections::HashSet};

use gpu_allocator::MemoryLocation;
use moonfield_app::prelude::World;
use moonfield_asset::{AssetId, AssetRevision, Assets, Handle};
use moonfield_rhi::{Buffer, BufferUsage, RenderDevice};

use crate::mesh::gltf::{MeshGltfError, import_gltf_mesh, parse_gltf_mesh};

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
#[derive(Clone)]
pub struct Mesh {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    /// What the mesh was loaded from (e.g. the glTF path), for display.
    source: Option<String>,
    aabb_min: [f32; 3],
    aabb_max: [f32; 3],
}

/// CPU mesh data copied into the render world with the source revision.
pub struct ExtractedMesh {
    /// Revision observed in the main-world asset store.
    pub revision: AssetRevision,
    /// Immutable CPU geometry used to prepare GPU buffers.
    pub mesh: Mesh,
}

/// Referenced mesh assets available to render-world preparation systems.
#[derive(Default)]
pub struct ExtractedMeshes(HashMap<AssetId, ExtractedMesh>);

impl ExtractedMeshes {
    /// Get an extracted mesh by asset id.
    pub fn get(&self, id: AssetId) -> Option<&ExtractedMesh> {
        self.0.get(&id)
    }

    /// Iterate all extracted meshes.
    pub fn iter(&self) -> impl Iterator<Item = (AssetId, &ExtractedMesh)> {
        self.0.iter().map(|(&id, mesh)| (id, mesh))
    }

    /// Number of extracted mesh assets.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no mesh assets are extracted.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Prepared render-world values keyed by source mesh id and revision.
pub struct PreparedMeshes<T>(HashMap<AssetId, (AssetRevision, T)>);

impl<T> Default for PreparedMeshes<T> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

/// Vertex and index buffers prepared for one extracted mesh.
pub struct GpuMesh {
    vertex: Buffer,
    index: Buffer,
    index_count: u32,
    _render_device: RenderDevice,
}

impl GpuMesh {
    /// Upload positions and indices into GPU buffers.
    pub fn new(
        render_device: &RenderDevice,
        positions: &[[f32; 3]],
        indices: &[u32],
    ) -> moonfield_rhi::Result<Self> {
        let device = render_device.device();
        let vertex = Buffer::new(
            device,
            std::mem::size_of_val(positions) as u64,
            BufferUsage::VERTEX,
            MemoryLocation::CpuToGpu,
        )?;
        vertex.upload(device, positions)?;
        let index = Buffer::new(
            device,
            std::mem::size_of_val(indices) as u64,
            BufferUsage::INDEX,
            MemoryLocation::CpuToGpu,
        )?;
        index.upload(device, indices)?;
        Ok(Self {
            vertex,
            index,
            index_count: indices.len() as u32,
            _render_device: render_device.clone(),
        })
    }

    /// Prepared vertex buffer.
    pub fn vertex(&self) -> &Buffer {
        &self.vertex
    }

    /// Prepared index buffer.
    pub fn index(&self) -> &Buffer {
        &self.index
    }

    /// Number of indices recorded for this mesh.
    pub fn index_count(&self) -> u32 {
        self.index_count
    }
}

/// Revision-matched GPU meshes owned by the render world.
pub type PreparedGpuMeshes = PreparedMeshes<GpuMesh>;

/// Upload changed extracted meshes and discard GPU buffers whose source asset
/// is no longer part of the render snapshot.
pub fn prepare_meshes(world: &mut World) {
    let Some(render_device) = world
        .get_resource::<RenderDevice>()
        .map(|render_device| (*render_device).clone())
    else {
        return;
    };
    let Some(extracted) = world.get_resource::<ExtractedMeshes>() else {
        return;
    };
    let mut prepared = world
        .get_resource_mut::<PreparedGpuMeshes>()
        .expect("PreparedGpuMeshes registered by RenderFeaturePlugin");
    prepared.retain_extracted(&extracted);

    for (id, extracted_mesh) in extracted.iter() {
        if extracted_mesh.mesh.positions().is_empty()
            || extracted_mesh.mesh.indices().is_empty()
            || !prepared.needs_prepare(id, extracted_mesh.revision)
        {
            continue;
        }
        match GpuMesh::new(
            &render_device,
            extracted_mesh.mesh.positions(),
            extracted_mesh.mesh.indices(),
        ) {
            Ok(mesh) => {
                prepared.insert(id, extracted_mesh.revision, mesh);
            }
            Err(error) => moonfield_log::error!("failed to prepare mesh {id:?}: {error}"),
        }
    }
}

impl<T> PreparedMeshes<T> {
    /// Whether `id` is missing or was prepared from an older revision.
    pub fn needs_prepare(&self, id: AssetId, revision: AssetRevision) -> bool {
        self.0
            .get(&id)
            .is_none_or(|(prepared_revision, _)| *prepared_revision != revision)
    }

    /// Insert or replace a prepared value for a source revision.
    pub fn insert(&mut self, id: AssetId, revision: AssetRevision, value: T) -> Option<T> {
        self.0
            .insert(id, (revision, value))
            .map(|(_, previous)| previous)
    }

    /// Get a prepared value by source asset id.
    pub fn get(&self, id: AssetId) -> Option<&T> {
        self.0.get(&id).map(|(_, value)| value)
    }

    /// Get a prepared value only when it matches `revision`.
    pub fn get_for_revision(&self, id: AssetId, revision: AssetRevision) -> Option<&T> {
        self.0
            .get(&id)
            .filter(|(prepared_revision, _)| *prepared_revision == revision)
            .map(|(_, value)| value)
    }

    /// Remove prepared values whose source assets are no longer extracted.
    pub fn retain_extracted(&mut self, extracted: &ExtractedMeshes) {
        self.0.retain(|id, _| extracted.0.contains_key(id));
    }

    /// Number of prepared values.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no values are prepared.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Incrementally copy mesh assets referenced by [`MeshRenderer`] entities
/// into the render world.
pub fn extract_mesh_assets(world: &World, render_world: &mut World) {
    let Some(assets) = world.get_resource::<Assets<Mesh>>() else {
        render_world.insert_resource(ExtractedMeshes::default());
        return;
    };

    let referenced: HashSet<Handle<Mesh>> = world
        .query::<&MeshRenderer>()
        .map(|(_, renderer)| renderer.mesh.0)
        .filter(|handle| assets.contains(handle))
        .collect();
    let mut extracted = render_world
        .remove_resource::<ExtractedMeshes>()
        .unwrap_or_default();
    extracted
        .0
        .retain(|id, _| referenced.iter().any(|handle| handle.id() == *id));
    for handle in referenced {
        let Some(revision) = assets.revision(&handle) else {
            continue;
        };
        if extracted
            .0
            .get(&handle.id())
            .is_some_and(|mesh| mesh.revision == revision)
        {
            continue;
        }
        let Some(mesh) = assets.get(&handle) else {
            continue;
        };
        extracted.0.insert(
            handle.id(),
            ExtractedMesh {
                revision,
                mesh: mesh.clone(),
            },
        );
    }
    render_world.insert_resource(extracted);
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
/// A plain newtype — a component through `moonfield-ecs`'s blanket
/// `Component` impl. Store the assets in the `Assets<Mesh>` world resource.
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

    #[test]
    fn test_prepared_meshes_invalidate_by_revision_and_prune_missing_assets() {
        let mut assets = Assets::<Mesh>::default();
        let first = assets.add(Mesh::new(Vec::new(), Vec::new(), None));
        let second = assets.add(Mesh::new(Vec::new(), Vec::new(), None));
        let first_revision = assets.revision(&first).unwrap();
        let second_revision = assets.revision(&second).unwrap();

        let mut extracted = ExtractedMeshes::default();
        extracted.0.insert(
            first.id(),
            ExtractedMesh {
                revision: first_revision,
                mesh: assets.get(&first).unwrap().clone(),
            },
        );
        let mut prepared = PreparedMeshes::default();
        assert!(prepared.needs_prepare(first.id(), first_revision));
        prepared.insert(first.id(), first_revision, "first");
        prepared.insert(second.id(), second_revision, "second");
        assert!(!prepared.needs_prepare(first.id(), first_revision));

        let _ = assets.get_mut(&first).unwrap();
        assert!(prepared.needs_prepare(first.id(), assets.revision(&first).unwrap()));
        prepared.retain_extracted(&extracted);
        assert_eq!(prepared.get(first.id()), Some(&"first"));
        assert!(prepared.get(second.id()).is_none());
    }
}

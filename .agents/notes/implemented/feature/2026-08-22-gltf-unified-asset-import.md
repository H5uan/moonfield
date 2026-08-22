# Agent Note: glTF as the unified asset source format

Status: implemented

[中文](2026-08-22-gltf-unified-asset-import.zh.md)

## Problem

The editor loaded only PLY splat clouds through a dedicated loader, and there
was no mesh asset at all — the viewport drew every `MeshRenderer` entity as a
colored unit cube, and `moonfield-render` carried a placeholder cube
`scene::MeshRenderer` (plus a serde dependency) whose only job was that
placeholder. Meanwhile the [scene save/load
system](../architecture/2026-08-21-bsn-style-scene-templates.md) had already
adopted glTF 2.0 as its text carrier, so the workspace maintained two
unrelated format stacks and still could not display a real mesh.

## Decision

glTF 2.0 (`.gltf`/`.glb`) is the engine's sole asset source format, parsed
with the full `gltf` crate (new workspace dependency, features `import` +
`utils`). Mechanisms live in
[docs/architecture.md](../../../../docs/architecture.md).

- `moonfield-renderer` gains `src/mesh/`: the `Mesh` asset (positions +
  indices behind accessors, a precomputed AABB, the source path — the
  `SplatCloud` shape), the `MeshHandle` component newtype, and the
  `MeshRenderer` component (`#[reflect(ignore)]` mesh, editable `color`).
  `mesh/gltf.rs` merges every TRIANGLE primitive in the file into one mesh,
  applying vertex offsets to the indices and synthesizing sequential indices
  for non-indexed primitives; POINTS primitives, node transforms, and
  materials are dropped.
- `splat/io/gltf.rs` replaces the deleted `ply.rs`: a
  `KHR_gaussian_splatting` (Khronos RC) loader reading POINTS primitives that
  carry `KHR_gaussian_splatting:*` attributes — float component types only,
  kernel `"ellipse"` only, no compression sub-extensions (SPZ). The loader
  converts glTF render-space values into the training-space conventions
  `GaussianScene` keeps: scale → ln, opacity → logit, quaternion xyzw → wxyz,
  degree-0 SH → `f_dc` verbatim, higher-degree SH transposed into the
  channel-blocked `f_rest` layout with missing degrees zero-filled.
  `SplatCloud::from_ply_*` becomes `from_gltf_file`/`from_gltf_bytes`, and
  `SplatLoadError` is now `{Io, Gltf}`. Because gltf-json maps the unknown
  extension semantics to `Checked::Invalid`, the splat loader parses via
  `Gltf::from_slice_without_validation` + `import_buffers` and reads the
  attribute map from the raw JSON; mesh loading uses validated
  `gltf::import`.
- `moonfield-render` gains depth support — `OffscreenTarget::new_with_depth`
  (D32Sfloat), `RenderPass::new_with_depth`, and `PipelineOptions.depth_test`
  (reverse-Z: clear 0.0, compare `GREATER_OR_EQUAL`) — and loses the
  placeholder cube `scene::MeshRenderer` together with the crate's serde
  dependency. `tests/depth_occlusion.rs` covers the reverse-Z path.
- `moonfield-scene` roundtrip-tests the editor's `mesh_renderer` registry
  entry: a path-string custom entry whose load hook wraps
  `HandleTemplate<Mesh>::Path` into the `MeshRenderer` newtype.
- The editor replaces `SplatCloudLoader` with `GltfLoader`, which sniffs the
  file bytes for the `"KHR_gaussian_splatting"` JSON key and produces a
  `SplatCloud` or a `Mesh` accordingly; `load_asset` (was `load_splat_cloud`)
  spawns the named entity with the matching component (a `MeshRenderer` in
  `DEFAULT_MESH_COLOR`). The viewport draws real meshes through an
  `AssetId → GpuMesh` cache into a depth-tested target and keeps the splat
  AABB placeholder on an internal unit-cube mesh. A latent row/column
  mismatch the old `// DEBUG bypass mvp` shader line had masked is fixed at
  its root: Slang packs push-constant matrices row-major while glam's
  `to_cols_array()` is column-major, so the viewport shaders declare
  `column_major float4x4 mvp;`.

## Alternatives considered

- **Keep PLY for splats and add glTF only for meshes.** Rejected: two source
  formats means two loaders and two failure surfaces, while glTF covers both
  asset types through one parser and one content-sniffing dispatch. The PLY
  loader is deliberately removed; training-side interop will later be served
  by a `KHR_gaussian_splatting` exporter, not by keeping an import-only
  second format alive.
- **Hand-roll the glTF container and accessor decoding.** Rejected:
  container parsing, external-buffer resolution, and accessor machinery are
  exactly what the `gltf` crate implements and tests; duplicating them buys
  nothing. The only decoding kept by hand is the splat attribute read, forced
  by gltf-json's `Checked::Invalid` mapping of the unknown extension
  semantics.
- **Preserve the glTF scene graph — node transforms, primitive splits,
  materials.** Deferred: faithful multi-primitive, multi-material import
  needs a Material system and per-primitive draw state the renderer does not
  have yet. v1 merges all triangle primitives into one flat-colored `Mesh`;
  the split arrives with the Material system.

## Consequences

- PLY files no longer load anywhere; existing splat captures must be
  re-exported as `KHR_gaussian_splatting` glTF.
- One glTF file yields one `Mesh`: primitive boundaries, node placement, and
  materials are flattened away at import, so files that only read through
  their materials come in flat-colored until the Material system lands.
- Splat import is strict: non-float (quantized) attributes, non-ellipse
  kernels, and SPZ-style compression sub-extensions are explicit errors, not
  silent degradation.
- `mesh_renderer` rides scene files as a bare path string; the color is not
  persisted — scene-loaded meshes come back in `DEFAULT_MESH_COLOR`.
- A begun depth pass takes two clear values (color, then depth 0.0), and any
  Slang shader reading a push-constant matrix must declare it `column_major`
  to match glam's layout.

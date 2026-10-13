# Agent Note: The fixed-function vertex path is gone - pulling is the only vertex story

Status: implemented

[中文](2026-09-05-no-input-assembler.zh.md)

## Problem

After the mesh pipeline pulled its geometry through pointers, the
fixed-function vertex path survived only as dead surface: `BufferUsage::
VERTEX`/`INDEX` vocabulary, `bind_vertex_buffers`/`bind_index_buffer`
commands, `draw_indexed` (all three variants), per-pipeline
`VertexBufferLayout` construction, and `Reflection::vertex_layout`
derivation - with one remaining production user, the egui backend, paying
the ceremony for data pointers already reach.

## Decision

- egui pulls like everything else: `vs_main(SV_VertexID, uniform
  EguiRoot root)` fetches `root.vertices[root.indices[vid +
  root.index_base]]`. The upload rewrites each mesh's local indices to
  absolute vertex indices, so a draw carries only its index range; the
  root's static prefix (32 bytes: screen size, flags, both array
  pointers) is pushed once per pass and the 16-byte tail (texture,
  sampler, index base) per draw. The frame slots hold host-visible
  `GpuAllocation`s (wholesale-rewritten every frame), not `Buffer`s.
- The RHI deletes the whole fixed-function surface: the types
  (`VertexBufferLayout`, `VertexAttribute`, `VertexFormat`,
  `IndexFormat`, `DrawIndexedIndirectArgs`), the commands
  (`bind_vertex_buffers`, `bind_index_buffer`, `draw_indexed` and both
  indirect variants), `Reflection::vertex_layout`, and the
  `vertex_layout` parameters of every `GraphicsPipeline` constructor.
  Pipelines emit no vertex input descriptions - the mesh-shader shape.
- `draw_indirect`/`draw_indirect_count` (non-indexed) remain; the
  indirect-draw test's indexed section became a two-record non-indexed
  `draw_indirect`, which exercises multi-draw args parsing (draw count
  and stride) the old path never did.

## Alternatives considered

- **Keeping the fixed-function path for exotic pipelines.** Nothing in
  the workspace needs it; keeping dead surface costs every reader the
  question "which vertex story applies here?".
- **Pulling indices but keeping the index-buffer bind.** Half a
  mechanism; the non-indexed draw with `index_base` in the root is the
  same one draw's worth of data with no bound state.

## Consequences

- A draw in this engine is exactly: pipeline bind, root-data pushes,
  `draw`. No bound vertex state exists to get wrong, and the pipeline's
  vertex input state is the same for every pipeline that will ever
  exist.
- A reflection ABI fact pinned along the way: the push-data bank is
  shared across stages, and each stage's entry signature is the ABI - a
  root declared only in one stage still occupies the bank from offset 0,
  so a stage with its own roots must declare the shared leading roots
  first (or the two stages' placements collide silently). The
  `bindless_graphics_heap_sampling` fragment stage declares the vertex
  stage's leading `Ptr` for exactly this reason; `egui.slang` and
  `core_3d.slang` already shared their signatures.
- All pixel tests pass unchanged on the real GPU: pulling through
  `first_vertex`-offset `SV_VertexID` reproduces the old draws exactly.

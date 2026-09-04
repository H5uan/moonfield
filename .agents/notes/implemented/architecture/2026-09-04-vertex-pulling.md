# Agent Note: Geometry through pointers — vertex pulling lands

Status: implemented

[中文](2026-09-04-vertex-pulling.zh.md)

## Problem

The mesh pipeline was the one non-bindless path left: `GpuMesh` owned a
`Buffer` pair bound through the fixed-function input assembler
(`bind_vertex_buffers` / `bind_index_buffer` per draw, plus a per-mesh
vertex layout baked into the pipeline), and the geometry lived in
host-visible memory (`Memory::Default`) written by a blocking upload. The
RHI is buffer-device-address-everywhere, so the vertex path paid fixed-
function ceremony for data that pointers already reach.

## Decision

- `DrawData` grows to the per-draw record's terminal shape: `{ model,
  color, positions: Ptr<float3>, indices: Ptr<uint>, index_count }` —
  everything one draw needs, behind one pointer. The vertex shader's only
  stage input is `SV_VertexID`; it pulls both arrays through the record's
  pointers (`vi = indices[vid]; position = positions[vi]`), and the draw
  becomes non-indexed (`draw(index_count, 1, 0, 0)`).
- `GpuMesh` is a pair of GPU-only `GpuAllocation`s; geometry stages
  through the shared frame uploader (`upload_alloc`, one flush per frame
  ahead of the frame command buffer — the same-queue ordering the egui
  uploads already ride).
- Pipelines accept an empty vertex layout: no binding or attribute
  descriptions are emitted — the pulling pipeline has no input
  assembler, the shape mesh-shader pipelines already use.
- `SV_VertexID` reflects with category `None` and its semantic name, so
  the varying-input filter excludes it from derived vertex layouts — no
  special-casing needed (`pulling_vertex_shape` pins this).

## Alternatives considered

- **A shared geometry arena before pulling.** Rejected ordering: pulling
  removes the per-draw binds on its own (shaders take pointers), while
  the arena only addresses allocation hygiene — a later, independent
  step.
- **Separate root parameters for the geometry pointers.** Rejected: one
  record per draw is the shape instancing and indirect draws consume;
  `Ptr` fields inside a uniform struct lay out at natural offsets
  (verified), so the record carries them.

## Consequences

- `DrawMesh` records a draw with one pipeline bind, one 8-byte push, and
  one `draw` — no vertex/index binds. The `BufferUsage::VERTEX`/
  `INDEX` vocabulary has no production user in the mesh path.
- Out-of-order teardown (GPU resources outliving the `RenderDevice`)
  crashed during this work: the retirement ring's pending actions hold
  the last `Arc<Allocator>`, and their drop frees memory through a
  destroyed device. `Device::drop` now leaks (device and allocator,
  with an error log) instead of destroying when allocation Arcs remain,
  and `Instance` tracks live devices through a shared counter so its
  own `Drop` leaks rather than destroying around a live device. The
  tests insert `RenderDevice` first, mirroring the real plugin order —
  the guards are the mechanism, the order is the convention.
- GPU-verified end to end: the opaque-pass pixel tests pass unchanged
  (the draw pulls the same geometry through the same transform math).

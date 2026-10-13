# Agent Note: Lightweight render-phase framework

Status: implemented

[中文](2026-08-26-render-phase-framework.zh.md)

## Problem

The core 3D pass recorded meshes directly: `main_opaque_pass_3d` imported
`ExtractedMeshes` and `PreparedGpuMeshes`, and the mesh queueing logic lived in
a hard-coded `Opaque3dPhase::queue` called by the camera driver while building
`Core3dFrame`. Adding a second draw kind (transparency, splat
rasterization) meant editing the pass and the frame structure.
[Renderer aligned with Bevy](2026-08-24-renderer-bevy-alignment.md) had
rejected a draw-function registry for the one-pass, one-phase frame; the
registry is adopted now that the pass must stop naming mesh types.

## Decision

`moonfield-render-core` (Selene) owns a minimal phase framework in
`render_phase.rs` — the Bevy `RenderPhase`/`DrawFunctions` shape without the
`RenderCommand` chain:

- `PhaseItem` — pure queued data with an `Ord` sort key and a `DrawFunctionId`.
- `DrawFunction<P>` — records one item's GPU work from `(&World, &P,
  &CommandBuffer)`.
- `DrawFunctions<P>` — a render-world resource; features register their draw
  functions once in plugin build, pass systems look items' ids up.
- `RenderPhase<P>` — one view's sorted item collection: `Default` (empty),
  `add`, `sort`, `items`.
- `OrderedFloat` — `Ord` wrapper for `f32` sort keys (`total_cmp`).

In `moonfield-render-feature` (Lunaris), the mesh feature registers `DrawMesh`
(pipeline + vertex/index binding + push constants + indexed draw, including
the revision-matched GPU-buffer check) into `DrawFunctions<Opaque3d>`, and its
`queue_opaque_3d` system fills each view's `RenderPhase<Opaque3d>` with
live-mesh items, computing camera-space depth and the final view-projection ×
model matrix at queue time. `Core3dFrame` keeps its camera-driver duties
(primary-view ordering, per-target grouping) and exposes `views_mut` for queue
systems; `build` no longer queues anything. The opaque pass clears
attachments, sets the pass-wide viewport/depth/cull state, and dispatches each
item to its registered draw function — it imports no mesh type.

## Alternatives considered

**Full Bevy `RenderCommand` chain with macro combinators and batching.**
Rejected: the command buffer is single-threaded and the frame has one
pipeline; the combinator machinery exists for parallel recording and
multidraw, which moonfield does not use.

**`enum Drawable` over all draw kinds, matched in the pass.** Rejected: the
pass would still name every draw kind and would need editing to add one.

**Item self-packaged draw closures.** Rejected: a registry keeps draw
functions in one place per phase (registerable once by features) and keeps
items `Copy` — Bevy's shape.

## Consequences

- Adding a draw kind is registration, not modification: a phase item type, a
  queue system, and a draw function, with the pass untouched.
- The pass no longer imports `ExtractedMeshes`/`PreparedGpuMeshes`; the mesh
  feature owns its pipeline and per-draw resources through `DrawMesh`.
- Queue systems run in `RenderQueue` ordered after `prepare_core_3d_frame`;
  `Core3dFrame::build` creates empty phases.
- Queue-time `mvp` uses the same `RenderTargetSizes`/initial-size fallback as
  `prepare_view_targets`, so projected geometry matches the prepared
  attachment.
- `RenderPhase` carries `Debug`/`Clone`/`PartialEq` derive bounds so
  `Core3dFrame`'s existing derives hold; `Default` is manual so a phase needs
  no `P: Default`.
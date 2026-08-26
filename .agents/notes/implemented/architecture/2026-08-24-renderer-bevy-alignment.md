# Agent Note: Renderer aligned with Bevy

Status: implemented

[中文](2026-08-24-renderer-bevy-alignment.zh.md)

## Problem

The application had a render world and handwritten extraction, but the render
schedule still executed against the main world. The editor owned CPU
interaction state, GPU resources, mesh upload, scene queueing, swapchain frame
control, and presentation inside one exclusive function. Render-world entities
were rebuilt each frame without a stable reference to their main-world source,
and mutable assets had no revision that could invalidate prepared GPU data.

That structure made the render world observational rather than authoritative:
the viewport still queried main-world assets and cameras while recording GPU
commands, and there was no render-world contract for camera views, prepared
meshes, opaque draw order, or the acquire → pass → submit frame boundary.

## Decision

`App::render` runs five explicit stages:

```text
PreRender(MainWorld)
→ clear render snapshot entities and extract MainWorld → RenderWorld
→ RenderPrepare(RenderWorld)
→ RenderQueue(RenderWorld)
→ Render(RenderWorld)
```

Main-world schedules register through `App::add_systems`; render-world
schedules register through `App::add_render_systems`. Render-world resources
survive snapshot clearing and own all persistent GPU state. `RenderDevice`
exists only in the render world.

The coarse render stages follow the local Bevy `0.20.0-dev` lifecycle without
copying its full `SystemSet` graph. `RenderPrepare` converts extracted CPU data
into persistent GPU resources, `RenderQueue` builds per-frame view and phase
work, and `Render` records and submits commands. Moonfield keeps these as
separate render-world schedules because its scheduler has no `SystemSet`
primitive and executes on one thread.

`HierarchyPlugin` also runs transform propagation in `PreRender`.
`editor_prepare` is ordered before `ensure_global_transforms`, followed by
`propagate_transforms`, so orbit-camera edits reach `GlobalTransform` before
the main-world snapshot is extracted.

Every extracted scene entity carries `MainEntity`, which is the stable key for
its main-world source. `moonfield-camera` owns the scene-facing `Camera`,
`PrimaryCamera`, `CameraTarget`, `RenderTarget`, and projection/view math without
depending on the Vulkan RHI. `moonfield-render-core` (Selene) consumes those
types and keeps the render-world `ExtractedView`, `ViewTarget`, and extraction
systems. Camera
extraction records the camera parameters, propagated transform, source
identity, and logical target. `CameraTarget` remains a separate runtime
component, so scene camera serialization is unchanged.

`Assets<T>` assigns an `AssetRevision` when an asset is inserted or mutably
accessed. Mesh and splat extraction copies only live assets referenced by
renderable entities. `ExtractedMeshes` persists in the render world and replaces
CPU geometry only when the revision changes. `PreparedMeshes<T>` associates GPU
data with the source `AssetId` and revision; stale or unreferenced entries are
not used for drawing.

The editor has two owners joined by a bounded bridge:

- `EditorMainState` lives in the main world and owns egui input, docking,
  selection, scene editing, orbit-camera, and gizmo state.
- `EditorRenderState` lives in the render world and owns `WindowRenderer`, the
  offscreen viewport, the egui Vulkan renderer, frame slots, and delayed texture
  destruction.
- `PreparedEditorFrame` carries the newest UI shapes and texture updates from
  `PreRender` to the render world. Render feedback carries the viewport texture
  id and completed-frame count in the opposite direction.

The render schedule drives the window frame as three ordered systems:
`editor_acquire`, `editor_record`, and `editor_submit`. Acquisition is the only
caller of `WindowRenderer::begin_frame`; submission is the only caller of
`WindowRenderer::end_frame`. A recording failure still reaches submission so
the acquired image and command-buffer state are closed before the next frame.

`moonfield-render-feature` is the high-level render-feature layer above the
`moonfield-rhi` RHI and the `moonfield-render-core` engine layer, analogous to
a Bevy feature crate such as `bevy_pbr`. Its
`RenderFeaturePlugin` prepares `PreparedGpuMeshes` from `ExtractedMeshes` during
`RenderPrepare`, then builds a `Core3dFrame` during `RenderQueue`. GPU mesh
buffers are keyed by source `AssetId` and `AssetRevision`, live independently of
the editor viewport, and are available to any render-world consumer. Each
`Core3dView` owns a sorted `RenderPhase<Opaque3d>`; the mesh feature's
`queue_opaque_3d` fills it with live-mesh items and registers `DrawMesh` in the
phase's `DrawFunctions` registry — the pass dispatches items to their
registered draw functions, so it never names mesh types (see
[render phase framework](2026-08-26-render-phase-framework.md)). The editor viewport consumes the primary viewport-targeted view
and records its already-prepared opaque phase into the persistent offscreen
target before the egui pass samples that target.

## Alternatives considered

**Full Bevy sub-world synchronization and retained render entities.** Rejected
because moonfield rebuilds a small render snapshot each frame. `MainEntity` plus
persistent resources supplies the identity needed by current caches without
observers, bidirectional maps, or `SubEntity` lifecycle machinery.

**A generic RenderAsset framework.** Rejected because the current renderer has
one prepared mesh path and one splat metadata path. Asset revisions and concrete
extraction caches cover invalidation without dependency graphs, upload budgets,
retry queues, or device-recovery policy.

**Split Vulkan window ownership into many ECS components.** Rejected because
`WindowRenderer` is the module that enforces swapchain, command-buffer, fence,
semaphore, surface, and device lifetime ordering. Systems drive its frame
boundary without exposing those internal ownership constraints.

**A render graph and full draw machinery.** Rejected for the shipped frame,
which has one Core3d scene pass and one editor UI pass: ordered systems, one
sorted phase per view, and `Core3dFrame` express the data flow without graph
nodes, binned phases, multidraw, batching, or a `RenderCommand` chain. A
lightweight draw-function registry was adopted separately to decouple the pass
from the mesh feature — see [render phase framework](2026-08-26-render-phase-framework.md).

**The complete Bevy 0.20 render lifecycle.** Rejected because `SubApp`
pipelining, retained render-entity synchronization, `RenderStartup` device
recovery, and the full `RenderSystems` set graph solve scale and threading needs
moonfield does not have. The separate render world and coarse schedules preserve
the applicable ownership and ordering boundaries without making Vulkan objects
`Send` or extending the ECS scheduler first.

**Vulkan work graphs or PBR in the same migration.** Rejected because neither
is required to establish world ownership, asset preparation, view extraction,
or pass ordering. The current opaque path keeps the existing flat-color shader
and CPU-recorded indexed draws.

**Keep camera APIs in `moonfield-rhi`.** Rejected because scene and editor
code should not obtain camera components from the Vulkan RHI. A standalone
camera crate keeps camera data and math reusable while allowing the RHI and
feature crates to consume it in one dependency direction.

## Consequences

Render command recording no longer reads main-world cameras or mesh assets.
Orbit-camera edits and pre-render transform propagation occur before extraction,
so the same frame's snapshot observes the updated transform. The render world
is the sole owner of Vulkan state, and the editor can run its main-world
preparation without a Vulkan device.

`Assets::get_mut` conservatively advances the asset revision even if the caller
does not change the value. This may cause an unnecessary re-upload, but it
prevents silent reuse of stale GPU data without adding mutation guards or asset
events.

The bridge holds only the newest prepared editor frame. Replaced frames merge
their texture updates so font and user-texture deltas are not lost when the
render side skips a frame. Minimized or out-of-date windows retain pending work
until acquisition succeeds.

The opaque phase covers flat-lit meshes only. It does not include materials,
transparency, shadows, automatic batching, or GPU-driven draws — new draw
kinds register a phase item, a queue system, and a draw function instead of
modifying the pass. `ViewTarget`
selects the primary window or editor viewport; the persistent offscreen Vulkan
target remains owned by `EditorRenderState`.

GPU mesh preparation belongs to `moonfield-render-feature`, while the viewport
continues to own its target and graphics pipeline.
Low-level Vulkan objects remain ordinary Rust owners or render-world resources;
ECS controls their lifecycle phases rather than decomposing them into entities.
Each prepared GPU mesh retains the shared device so buffer destruction does not
depend on the render world's resource drop order.

The render-feature crate's default feature is `mesh`; `splat` is opt-in and
depends on `mesh`. The `rt` and `gi` placeholder modules are removed. This
establishes the layer boundary without prematurely splitting each algorithm
into a separate crate; feature-specific crates such as `moonfield-pbr` can be
introduced when their public contracts exist.

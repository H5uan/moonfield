# Agent Note: Render pass schedule redesign

Status: implemented

[中文](2026-09-09-render-pass-schedule-redesign.zh.md)

## Problem

The pass structure of a frame lived inside one system. `main_opaque_pass_3d` in `moonfield-render-feature` lazily built the single `Core3dPipeline`, iterated views, and recorded the opaque pass inline; ordering against neighboring systems was anchored by function names, and the editor coupled to those names at compile time. Adding one compute pass in front of the opaque pass touched about eight files across four crates; a post-process pass touched six. One view per target rendered — the first primary view — and the opaque pass discarded its depth attachment, so nothing downstream could depth-test.

The phase machinery ([render-phase framework](2026-08-26-render-phase-framework.md)) carried the per-frame costs and single-pipeline limits the [superseded optimization note](../../rejected/architecture/2026-08-31-render-function-optimization.md) documents: HashMap dispatch, per-item resource fetches, sorting inside the queue system, and a per-frame `Core3dFrame` clone.

Extraction was a registration-ordered closure list on `App`, and schedules were `App` fields — no system could run a schedule, and extraction could not be ordered or filtered.

The [Gaussian Splatting roadmap](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md) M3 requires recording the tile-based forward as a pass between asset upload and the view target. The old shape had no home for a compute pass chain, no per-view execution, and no attachment sharing; this redesign exists to host M3.

## Decision

The renderer runs on the schedule model Bevy 0.20-dev converged on after deleting its `RenderGraph` (its slot system saw no use; schedules serve as sub-graphs):

- **Schedules are world data.** moonfield-ecs holds a `Schedules` resource and `World::run_schedule(label)` as the only run primitive; `App` methods are wrappers. Exclusive systems (`fn(&mut World)`) and `SystemState<P>` live in the same crate ([schedules as world data](2026-09-09-schedules-as-world-data.md)).
- **Extraction is a schedule.** `App::render` parks the main world in a `MainWorld(World)` resource, clears render-world entities (resources persist), runs `ExtractSchedule` — systems read the main world through an `Extract<T>` parameter — swaps the world back, then runs `Render`. The closure list is gone; the per-frame entity rebuild stays ([extract schedule](2026-09-09-extract-schedule.md)).
- **One `Render` schedule, set-ordered.** The frame sequencer's acquire is the non-public entry; then `PrepareAssets` → `Queue` → `PhaseSort` → `PrepareViews` → `CameraDriver` → `PostViews` → `Submit`. The `RenderPrepare` and `RenderQueue` labels are gone.
- **Per-view execution.** `CameraDriver`, an exclusive system, orders views by `(Camera::order, entity)` — `Camera` carries `order` — points `CurrentView` at each, and runs that view's schedule. `Core3d` is the one registered view schedule, holding the anchor set `Core3dOpaquePass`; the splat chain `[sort → forward → composite]` anchors `.after(Core3dOpaquePass)`, which is also the order the eventual mesh-plus-splat depth composite needs ([set chain and per-view schedule](2026-09-09-render-set-chain-and-per-view-schedule.md)).
- **`RenderContext` is a closed recording surface.** A `SystemParam` with three typed doors: `begin_rendering(RenderingDesc) -> TrackedRenderPass`, `compute() -> ComputeRecording`, and `barrier(before, after, hazard)`. No raw command-buffer handle; device access composes as separate `Res` parameters. A stage state machine inserts the rhi's global, resource-less barriers automatically — on door switches, and `COMPUTE → COMPUTE` between consecutive dispatches; draws inside one rendering need none; a manual `barrier` records as given and updates the machine. The frame command buffer stays single ([render recording context](2026-09-09-render-recording-context.md)).
- **Attachments are view-entity components over persistent maps.** GPU resources stay in resource maps (`ViewTargets`, `WindowSurfaces`); view entities carry the per-frame linkage, and load/store is declared per pass. The depth store strategy is an explicit `Core3d` decision ([per-view attachment components](2026-09-09-per-view-attachment-components.md)).
- **Draw machinery is `Vec`-indexed** (absorbing the superseded note): `Vec`-indexed dispatch with phase-typed `DrawFunctionId<P>`, `RenderCommand<P>` plus `RenderCommandState` over `SystemState`, a unified `PipelineCache` — graphics keyed `(ShaderHandle, GraphicsStateKey)`, compute keyed `ShaderHandle`, created synchronously in `PrepareViews` — and a thin `SortedPhasePlugin<P>` registration surface. `Core3dFrame` is dissolved into view components.
- **The GS forward renders to its own image.** The blend kernel writes an RGBA16F storage image (one additive rhi constructor, `SAMPLED|STORAGE`); a composite pass tonemaps into the view target. Per-view artifacts (tile lists, transmittance) live in a `SplatViewArtifacts` resource map keyed like `ViewTargets`.
- **Editor decoupling.** The editor overlay anchors the `PostViews` set instead of render-feature system names.

## Alternatives considered

- **A runtime render-graph object.** Lost: the reference implementation deleted its own graph runner — the slot system saw no use and schedules already serve as sub-graphs — and a graph runner is a second execution engine beside the ECS scheduler, with node state machines and runtime slot matching to maintain.
- **Keep three render schedules with sets inside.** Lost: schedule labels and sets would be two spellings of one ordering concept; per-view schedule runs still require schedules as world data, so the split saves nothing and defers the machinery M3 needs.
- **`PendingCommandBuffers` now.** Lost: moonfield-ecs runs systems serially, each with exclusive world access, so deferred encoder finishing has no benefit; it would also reorder the [frame command buffer](2026-09-06-frame-context-owns-frame-command-buffer.md) submit path that the [timeline pacing](2026-08-28-timeline-frame-loop.md) owns.
- **A raw `cmd()` door on `RenderContext`.** Lost: Bevy's `command_encoder()` escape hatch is safe because wgpu synchronizes automatically; moonfield's RHI is raw Vulkan over bindless GPU pointers, where a raw door re-exports the whole command-buffer surface and leaves barrier discipline to every call site. Closed typed doors keep sync visible and the recording swappable.
- **Per-resource automatic barriers.** Lost: bindless descriptors are raw GPU pointers — the rhi barrier is deliberately resource-less, so wgpu-style read/write tracking is not expressible without redesigning the resource model. Stage-scoped automatic barriers are the form that fits.
- **Manual-only barriers between dispatches.** Lost: every current and planned compute chain (the radix sort passes, projection → bucketing → sort → blending) is a true read-after-write chain; leaving the highest-frequency sync case manual forfeits most of the automation's value.
- **GS forward writes the viewport target directly.** Lost: the offscreen color is `COLOR_ATTACHMENT|SAMPLED` — no storage usage, 8-bit precision, no tonemap point; the RGBA16F intermediate matches the reference implementation's float pipeline at the cost of one additive rhi constructor.
- **Batching, binned phases, a mesh transparent phase, more view schedules.** Lost: no customer for any of them; the superseded note and this redesign both decline machinery without a caller.

## Consequences

- A pass is a new file plus one registration call, ordered around the opaque pass with no edits to render-feature core — GPU-verified end to end by the [acceptance run](2026-09-09-render-pass-schedule-acceptance.md), which also records the one deliberate gap (`in_set` membership is fixed at registration, so the splat chain anchors on the opaque pass system).
- The editor viewport, PrimaryWindow direct-draw, and egui composite render unchanged.
- The GS forward chain has its home: per-view systems in `Core3d`, the artifacts map, the composite pass — the [GS roadmap](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md) M3 wording matches the shipped shape.
- `Schedules` in-world, `World::run_schedule`, exclusive systems, and `SystemState` are the ECS foundation render commands also build on; the extract closure list is gone.
- Queue systems must order before `PhaseSort`; a mis-ordered queue system silently renders unsorted content — the hazard the superseded note records.
- The automatic barriers are conservative global memory barriers; over-synchronization is possible, and `Stage` carries no attachment-output constant if attachment-load hazards surface in composite or overlay ordering.
- Pass recording is CPU-serial on one command buffer; if the editor or the GS workload grows past it, the parallel-executor decision reopens.

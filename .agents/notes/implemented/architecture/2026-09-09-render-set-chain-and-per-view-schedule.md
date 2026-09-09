# Agent Note: Render set chain and per-view schedule

Status: implemented

[中文](2026-09-09-render-set-chain-and-per-view-schedule.zh.md)

## Problem

The `Render` schedule ordered its systems by function-name constraints: `main_opaque_pass_3d` was one monolith that lazily built the pipeline, iterated views itself, and rendered only the first primary view per target; sorting ran inside the queue system; the `Core3dFrame` resource was rebuilt per frame to carry the per-view phases; and the editor anchored on render-feature system names at compile time. Adding a pass meant editing that monolith.

## Decision

- moonfield-ecs gains `SystemSet`: a set is a typed no-op anchor system; `add_sets((A, B, ...))` registers the anchors as an ordered chain, `in_set::<S>()` attaches a system after its set's anchor and before the next anchor (so neighboring sets stay ordered without explicit constraints), and `before_set` / `after_set` are the one-sided forms.
- The `Render` schedule is one set chain — `PrepareAssets` → `Queue` → `PhaseSort` → `PrepareViews` → `CameraDriver` → `PostViews` → `Submit` — registered by `RenderPlugin`, with `acquire_window_frames` before the chain and `submit_window_frames` after it. The `RenderPrepare` and `RenderQueue` labels are gone.
- Phases are view-entity components: `RenderPhase<Opaque3d>` on each extracted view, attached by `prepare_view_phases` (`Queue`), filled by `queue_opaque_3d` (which no longer sorts), and sorted by the generic `sort_phase::<P>` system (`PhaseSort`). `Core3dFrame` and `Core3dView` are gone.
- `camera_driver<L>` (render-core) orders views by `(Camera::order, entity)` — `Camera` gains `order` — and runs the view schedule once per view with `CurrentView` inserted. The one instantiation today is `Core3d`, whose anchor set is `Core3dOpaquePass`; `opaque_pass_3d` is a per-view system inside it. Pipeline (re)building, view-target attachment, and the draw arena's per-frame begin moved to `PrepareViews` systems.
- Every view renders — the first-primary-view-per-target special case is gone. Offscreen targets no view claims are cleared to the background by `clear_orphan_view_targets` (after the driver, before `PostViews`).
- The editor anchors the `PostViews` and `Submit` sets instead of render-feature system names.

## Alternatives considered

- **Explicit two-sided attachment (`before_set(X).after_set(Y)`).** Lost: forgetting one side silently under-orders a system; `in_set` derives both edges from the chain, so membership is one decision.
- **Bevy-style set graph nodes.** Lost: the schedule is a topological sort over labeled systems; a typed anchor *is* a node, and the chain encodes the edges — no new execution semantics.
- **Keep `Core3dFrame` as the phase carrier.** Lost: the pass stays a centralized iterator over a rebuilt resource instead of per-view systems over view data — exactly the shape the [redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) removes.
- **A parameter-driven (non-exclusive) per-view pass via `ViewQuery`.** Lost: `DrawFunction::draw` still takes `&World` — parameterizing the pass now means redesigning the draw path, which is M3's `RenderCommand` work. `ViewQuery` ships for the first parameterized consumer.

## Consequences

- Multi-camera scenes render every view into its target in `(order, entity)` sequence; a later view's clear overwrites an earlier view on the same target, matching camera-overlay semantics.
- `ViewQuery` and `CurrentView` are public render-core surface; the opaque pass is exclusive (the draw functions read the world) until M3.
- Editor ordering no longer references render-feature internals; splat's chain anchors on `Core3dOpaquePass` when it lands.

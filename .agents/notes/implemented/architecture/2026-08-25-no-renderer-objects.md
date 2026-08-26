# Agent Note: No renderer objects — data-driven frame orchestration

Status: implemented

[中文](2026-08-25-no-renderer-objects.zh.md)

## Problem

The render stack grew around owning objects: `WindowRenderer` drove the
swapchain frame loop through a `begin_frame`/`end_frame` method contract, the
editor's `Viewport` owned a pipeline and recorded the scene pass itself, the
egui backend was a single `EguiRenderer`, and `EditorRenderState` glued all
three into a god object mutated by three hand-ordered systems through a
take-out/put-back slot. Bevy — the architecture this workspace follows — has
no `Renderer` type at all (checked against bevy 0.20-dev): GPU singletons are
flat resources, per-window state is ECS data, and the frame flow is a schedule
of systems. The object shape hid the frame orchestration from the schedule,
forced debug seams (`MOONFIELD_EDITOR_SCENE_ONESHOT`, `..._SKIP_UI`) to exist,
and blocked the bevy-style direction (pipelined rendering needs a render world
that is pure data).

## Decision

No type named `*Renderer` owns a frame loop anywhere; rendering is resources +
components + systems.

- **Window frames** (`moonfield-render-core/src/window.rs`): per-frame window
  snapshots are extracted as `ExtractedWindow` components
  (`extract_windows`); persistent surface/swapchain/sync state lives in the
  `WindowSurfaces` resource keyed by `MainEntity` (the render world clears
  entities every frame, so persistent GPU state must be a resource). The frame
  loop is three public systems — `create_window_surfaces` (RenderPrepare),
  `acquire_window_frames` and `submit_window_frames` (Render) — registered by
  `RenderPlugin` as ordering anchors other plugins chain `.after()`/`.before()`.
  Acquire is gated by the `WindowFrameDemand` resource (written during
  extraction) and skips zero-size windows, so nothing presents an image no
  pass recorded into.
- **Scene pass** (`moonfield-render-feature/src/core_3d/pass.rs`): the
  flat-lit pipeline is the lazily-created `Core3dPipeline` resource, offscreen
  attachments are the `ViewTargets` resource sized via `RenderTargetSizes`
  (written by the editor's extraction), and `main_opaque_pass_3d` is a plain
  `Render` system recording every view's `RenderPhase<Opaque3d>` into the window
  frame's command buffer.
- **Editor** (`moonfield-editor`): `EditorRenderState`, `EditorBridge`, and
  `Viewport` are gone. The main world stages a `PendingEditorFrame` resource;
  `extract_editor_frame` moves it into the render world (merging into an
  unconsumed frame so egui texture deltas are never dropped) and sets
  `RenderTargetSizes` + `WindowFrameDemand`. `EguiRenderer` split into three
  render-world resources — `EguiPipeline` (pipeline, layouts, sampler cache,
  `EguiOptions`), `EguiTextures` (texture map, deferred-free ring, upload
  pool), `EguiFrameResources` (per-slot buffers) — driven by the
  `prepare_egui_frame` → `egui_pass` → `editor_frame_done` systems ordered
  against the window and scene anchors. Render→main feedback (viewport texture
  id, presented-frame count) is the one remaining channel, an
  `EditorFeedbackChannel` `Arc` cloned into both worlds.
- **Resource teardown is LIFO** (`moonfield-ecs`): `World`'s resource store
  drops in reverse first-insertion order. Vulkan wrappers hold raw `ash`
  handles, so GPU objects created from an earlier-inserted `RenderDevice`
  must be destroyed before it; HashMap drop order caused an access violation
  at shutdown until this was made deterministic.

## Alternatives considered

- **Keep the objects, rename them.** Rejected: the problem was never the name
  but that frame orchestration lived in method contracts invisible to the
  schedule; renaming leaves the take/put slot and the debug seams in place.
- **Retain render-world entities and put window state in components
  (bevy 0.20 exactly).** Deferred: `App::render` clears render-world entities
  every frame, so persistent GPU state cannot live on entities today; the
  `MainEntity`-keyed resource map matches bevy's pre-0.20 `WindowSurfaces`
  shape and entity retention is its own project (it also unblocks pipelined
  rendering).
- **Introduce system sets for ordering.** Deferred: `before`/`after` on
  public system functions covers today's graph; sets can join when a third
  party needs to order against a group.
- **Make every Vulkan wrapper hold `Arc<Device>` instead of relying on LIFO
  drop.** Deferred: the LIFO store fixes the observed shutdown crash with one
  local change and mirrors Rust's struct-field drop order; per-object `Arc`s
  remain the hardening option if resource insertion order ever becomes
  unreliable for this.

## Consequences

- The frame flow is readable as data: extract → prepare → queue →
  acquire → passes → submit, each a named system any plugin can order
  against. `MOONFIELD_EDITOR_SCENE_ONESHOT` and `MOONFIELD_EDITOR_SKIP_UI`
  are deleted; `MOONFIELD_EDITOR_DUMP_VIEWPORT` survives as a system.
- Plugins compose by registering systems and resources (`RenderFeaturePlugin`
  adds `main_opaque_pass_3d` without the editor knowing), the shape splat/rt/gi
  features should copy.
- Consumers must respect two contracts the type system cannot: persistent GPU
  state lives in resources (entities are rebuilt per frame), and resources
  created from `RenderDevice` must be inserted after it so LIFO drop destroys
  them first.
- `MeshRenderer` keeps its name — it is a per-entity component (bevy's
  `Mesh3d` analogue), not a frame-owning object.

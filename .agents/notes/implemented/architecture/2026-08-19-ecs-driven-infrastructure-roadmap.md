# Agent Note: ECS-driven infrastructure roadmap

Status: implemented

[中文](2026-08-19-ecs-driven-infrastructure-roadmap.zh.md)

## Problem

Engine infrastructure grew ad hoc: the app ran flat system vectors with no
ordering, the editor owned the whole renderer, and nothing in the scene was
inspectable — no hierarchy, no time, no assets, no way for the editor to see
or edit what the game-visible world contained. The project needed its own ECS
architecture line rather than a third-party dependency and, at the same time,
an editor that can inspect and edit everything the engine surfaces.

## Decision

Drive all engine infrastructure through the ECS, in the style of a mainstream
retained-mode ECS, with a dual goal: (a) our own ECS architecture line, and
(b) making everything inspectable/editable by the editor. Scope is the
**middle layer**: the game-visible surface (scene entities, Transform
hierarchy, camera, time), an asset system, and ECS mirrors of render
resources. Explicitly out of scope: a separate render world / extract split —
that belongs to the future multi-threaded rendering effort.

Method: port runtime mechanisms from the local checkout of the reference
implementation, borrowing at the architecture level rather than mirroring its
API. No proc-macro crates, with one sanctioned exception
(`moonfield-reflect-derive`); `Component`/`Resource` stay blanket impls;
system params use hand-written impls plus tuple macros; the schedule keeps a
single-threaded executor only. [docs/architecture.md](../../../../docs/architecture.md)
carries the resulting runtime mechanisms.

The roadmap landed as eight milestones:

1. **ECS core** — system params (`Res`/`ResMut`/`Query`/`Local`/`Commands`),
   labeled schedules with `before`/`after` ordering (stable topological sort,
   single-threaded), commands drained after every system; `App` drives
   `Startup`/`Update`/`Render`/`Shutdown`, and the `AppExit` resource replaces
   the old `-> bool` exit convention.
2. **Component hooks** — `on_add`/`on_insert`/`on_discard`/`on_remove` (plus
   `on_despawn`) per component type; discard fires before the structural
   change, the rest after; a running hook is taken out of the registry to
   prevent same-hook recursion.
3. **Relationships and hierarchy** — generic `Relationship`/
   `RelationshipTarget` kept in sync by hooks; `ChildOf`/`Children` with
   linked-spawn recursive despawn and a panic on cycle-closing inserts;
   `Transform`/`GlobalTransform` in `moonfield-math`, propagated by
   `HierarchyPlugin` systems in `Update`.
4. **Time** — `Time<Real>`/`Time<Virtual>`/generic `Time` in `moonfield-time`
   (pause, relative speed, `max_delta` clamp on the virtual clock); the
   backend advances the clocks once per frame before `App::update`, lazily
   inserting missing ones.
5. **Render seam** — `RenderPlugin` creates the Vulkan instance and device
   and inserts them as the shared `RenderDevice` world resource
   (headless-tolerant); `WindowRenderer`/`EditorState` keep only window-bound
   and editor-only objects; render-phase systems query the `World` directly,
   with no extract layer.
6. **Scene-panel slice** — the editor viewport renders the ECS scene
   (`Camera`/`PrimaryCamera`/`MeshRenderer` in `moonfield-render::scene`)
   into the offscreen target; Hierarchy panel (entity tree from
   `ChildOf`/`Children`, `Name` labels, selection) and Inspector panel
   (auto-generated from `InspectorRegistry`); the
   `MOONFIELD_EDITOR_AUTO_CLOSE` smoke test.
7. **Mini reflection** — the `Reflect` trait (named-field enumeration,
   dynamic read/write, `Any` downcasts for leaves) plus `#[derive(Reflect)]`
   in `moonfield-reflect-derive`; the inspector walks it generically. No
   `DynamicStruct`, type registry, or serialization.
8. **Assets, sync first** — the zero-dependency `Assets<T>` slot-map store
   and the index+generation `Handle<T>`; `SplatCloud` is the first asset,
   loaded synchronously by the caller; training state stays outside the
   `World`.

## Alternatives considered

- **Depending on a full third-party ECS.** Rejected: the dual goal includes
  owning the architecture line — hooks, relationships, and the schedule are
  the seams the editor and the future render split build on, and a
  third-party core would make those seams someone else's API. It would also
  pull in a proc-macro-heavy stack the workspace deliberately avoids.
- **Render-world separation now.** Rejected (deferred): without
  multi-threaded rendering there is nothing to extract for, and a two-world
  split would double every scene type for zero current benefit. The
  single-threaded render seam — render-phase systems querying the `World`
  directly — keeps the later split possible.
- **API-level mirroring of the reference implementation.** Rejected:
  mirroring its API surface would import its complexity budget wholesale
  (derives, change detection, type registry). Architecture-level borrowing
  takes the semantics — schedules, hooks, relationships — and leaves the
  surface native to this workspace.

## Consequences

- Everything game-visible is now ECS data the editor can inspect and edit:
  the Hierarchy panel shows the live entity tree, the Inspector edits any
  registered component, and time/camera/assets are ordinary resources and
  components.
- The renderer no longer belongs to the editor: the shared `RenderDevice`
  resource serves the game and editor paths alike, and headless runs degrade
  to no-device instead of panicking.
- Dependency directions stay acyclic by construction: math types know no ECS,
  the renderer crate knows no ECS, and reflection sits below both
  (`moonfield-reflect` depends on glam directly to avoid a math↔reflect
  cycle).
- Known debts, recorded but not scheduled: the render-world/extract split, an
  async `AssetServer` plus task pool, real splat rasterization in the editor
  viewport, a multi-threaded executor, full observers, full reflection
  observers, full reflection
  (`DynamicStruct`), strong/weak handle refcounting, and the
  audio/physics/serialization/networking layers.

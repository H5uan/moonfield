# Agent Note: Render command dispatch

Status: implemented

[中文](2026-09-09-render-command-dispatch.zh.md)

## Problem

The draw dispatch carried the costs the [superseded optimization note](../../rejected/architecture/2026-08-31-render-function-optimization.md) documented: `DrawFunctions` was a `HashMap<u32, …>` paying a hash lookup per item per frame; `DrawFunctionId` was an untyped `u32`, so an id minted by one phase's registry compiled when passed to another's; `Opaque3dDrawFunction` threaded the id from plugin build to the queue system as a hand-written newtype resource; and `DrawMesh::draw` re-read its four resources from the world per item with no bind deduplication (every item re-bound the same pipeline).

## Decision

- `DrawFunctions<P>` stores a `Vec` of `(TypeId, Box<dyn DrawFunction<P>>)`; dispatch indexes by `DrawFunctionId<P>` — no hash. `DrawFunctionId<P>` carries `PhantomData<fn() -> P>`, binding every id to its phase at compile time.
- Registration and lookup are type-directed: `register::<C>()` stores the command's `TypeId`; `id::<C>()` recovers the id by type, so the queue system reads `DrawFunctions<Opaque3d>` and asks for `DrawMesh` directly. The `Opaque3dDrawFunction` newtype is gone.
- The stateless `RenderCommand<P>` trait declares `type Param: SystemParam` and an associated `render(world, item, pass, param)`; `RenderCommandState<P, C>` holds a `SystemState<C::Param>` and is the object-safe `DrawFunction<P>` the registry stores. `DrawMesh` is now a `RenderCommand` whose `Param` is the extracted meshes, prepared meshes, pipeline, and draw arena — fetched through the param once per item.
- `TrackedRenderPass` (render-core) wraps the frame command buffer and skips redundant pipeline binds. Tracking resets at every `begin_rendering`; a pipeline's address identifies it for the tracking window, sound because pipelines are immutable resources while a pass records (rebuilds happen in `PrepareViews`).
- `prepare_phase<P>` (generic phase attachment) and `sort_phase<P>` are bundled in `SortedPhasePlugin<P>`: one plugin per phase registers the view-phase plumbing in `Queue` and `PhaseSort`. `RenderFeaturePlugin` instantiates it for `Opaque3d`.

## Alternatives considered

- **Bevy's `SystemParam` read-only constraint on `Param`.** Lost: enforcing it needs a parallel `ReadOnlySystemParam` hierarchy in moonfield-ecs for one current caller; `Param` is fetched from `&World` and every resource the commands read is already read-only. Revisit if a command wants mutation.
- **Per-item pipeline identity through an rhi `id()` accessor.** Lost: it reopens the rhi API for a dedup key; the address identity is private to `TrackedRenderPass` and resets per pass.
- **The superseded note's `PipelineCache`.** Deferred, not dropped: the registry generalizes to many commands, but there is exactly one graphics pipeline today (`Core3dPipeline`, rebuilt on shader revision in `PrepareViews`); a keyed cache would hold one entry and its key type would be speculative. The first real customer — GS compute kernels or multi-material — brings the cache with a concrete key.
- **The superseded note's `prepare` hook on draw objects.** Dropped: `SystemState` in `RenderCommandState` already carries per-command persistent state; the hook had no caller.

## Consequences

- Phase items carry `DrawFunctionId<Opaque3d>`; queue systems resolve ids through the registry instead of a newtype resource.
- Draw commands are unit structs implementing `RenderCommand` — stateless, composable, and testable without a registry (the registry test pins typed lookup and index dispatch).
- Features without `RenderPlugin` (tests) get no set anchors, so ordering falls to registration order — the queue/sort test composes `RenderPlugin` to mirror the real app.

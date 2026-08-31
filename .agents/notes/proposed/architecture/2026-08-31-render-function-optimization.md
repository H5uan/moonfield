# Agent Note: Render-function pattern optimization (SystemParam dispatch, pipeline cache, tracked pass)

Status: proposed

[中文](2026-08-31-render-function-optimization.zh.md)

## Problem

The render-function machinery — `PhaseItem` / `DrawFunction<P>` / `DrawFunctions<P>` / `RenderPhase<P>` in `moonfield-render-core/src/render_phase.rs`, instantiated once as `Opaque3d` + `DrawMesh` in `moonfield-render-feature/src/render_phase.rs` (see the [render-phase framework](../../implemented/architecture/2026-08-26-render-phase-framework.md)) — carries per-frame costs and blocks multi-material use:

- `DrawFunctions<P>` stores draw functions in a `HashMap<u32, Box<dyn DrawFunction<P>>>`; every phase item pays one hash lookup per frame in the dispatch loop.
- `DrawMesh::draw` performs three `world.get_resource` lookups and two `HashMap` lookups per item per frame, none hoisted out of the item loop.
- Every item rebinds the graphics pipeline, vertex buffer, index buffer, and push constants with no deduplication; items sharing a pipeline re-issue identical bind calls.
- `Opaque3d` carries no pipeline field; `DrawMesh` fetches a single shared `Core3dPipeline` resource, so one opaque phase cannot hold more than one material.
- `DrawFunctionId(u32)` carries no `PhaseItem` type parameter; an id minted by `DrawFunctions<Opaque3d>` compiles when passed to another phase's registry, and it reaches the queue system through a hand-written `Opaque3dDrawFunction` newtype resource.
- `main_opaque_pass_3d` clones the entire `Core3dFrame` (all views and their phase `Vec`s) each frame to avoid a borrow conflict with `WindowSurfaces`.
- `view.opaque.sort()` is called from inside the queue system, so "items are sorted when the pass runs" has no single owner.
- `Core3dPipeline` is created lazily inside a `Render` system via a `contains_resource` guard with side effects.

The machinery is generic over `P` but exercised by one phase item and one draw function. The target scale is a few materials plus alpha and transparent phases; full PBR with GPU-culled multidraw is out of scope.

## Proposal

Align the dispatch path with Bevy's `RenderCommand` model where moonfield-ecs already provides the prerequisite `SystemParam` machinery, without adopting Bevy's full surface. The work is phased by dependency; each phase leaves the workspace building and tested.

### Dispatch storage and id safety

`DrawFunctions<P>` becomes a `Vec<Box<dyn DrawFunction<P>>>` indexed by `DrawFunctionId`; lookup is `O(1)` with no hash. `DrawFunctionId<P>` carries `PhantomData<P>`, so an id is bound to its phase at compile time. `DrawFunctions<P>::id::<T>()` recovers an id by `TypeId`, removing the hand-written `Opaque3dDrawFunction` newtype resource.

### SystemParam-driven render commands

A stateless `RenderCommand<P>` trait replaces the stateful `DrawFunction<P>::draw(&self, …)`. It declares `type Param: SystemParam` and `fn render(world, item, pass, param)`. A new `SystemState<P>` in moonfield-ecs — the `init_state` + `fetch` pair already used by `FunctionSystem`, lifted into a reusable container — caches the param state. `RenderCommandState<P, C>` wraps a `SystemState<C::Param>`, implements the object-safe `Draw<P>`, and is stored as `Box<dyn Draw<P>>`. At draw time `param = state.get(world)` fetches all resources once; the per-item `get_resource` and `HashMap` calls disappear. `Param` is constrained to read-only access so a render command cannot mutate the render world. An empty-default `fn prepare(&mut self, world)` hook is retained for future per-phase warmup.

### Pipeline cache and per-item pipeline

A `RenderPipelineCache` maps a (shader, static state) key to a `CachedRenderPipelineId`. `PhaseItem` gains `cached_pipeline()` as an opt-in trait; `Opaque3d` carries a pipeline id. `DrawMesh` binds the pipeline from `item.cached_pipeline()` rather than a singleton resource, and the queue system assigns the id per material. One opaque phase then holds multiple materials.

### Tracked render pass

A `TrackedRenderPass` in moonfield-render-core wraps `&CommandBuffer` plus a `DrawState` cache of the current pipeline, vertex buffer, index buffer, and bind groups. Each `set_*` short-circuits when the state is unchanged; `reset_tracking()` invalidates the cache when a render pass begins. Draw functions take `&mut TrackedRenderPass` instead of `&CommandBuffer`, so repeated binds across sorted items sharing a pipeline are elided.

### Sort ownership and pipeline creation

A `PhaseSort` schedule step owns sorting every phase; queue systems add items only. `Core3dPipeline` construction moves to `RenderPrepare`. `main_opaque_pass_3d` reads `Core3dFrame` without cloning it.

## Acceptance criteria

- [ ] `DrawFunctions<P>` stores `Vec<Box<dyn DrawFunction<P>>>`; `get(id)` indexes by `DrawFunctionId` with no `HashMap`.
- [ ] `DrawFunctionId<P>` is parameterized by its phase; `DrawFunctions<P>::id::<T>()` returns the id by `TypeId`; the `Opaque3dDrawFunction` newtype resource is removed.
- [ ] A `PhaseSort` schedule step sorts every phase; no queue system calls `sort()`.
- [ ] `Core3dPipeline` is constructed in `RenderPrepare`; no `Render` system creates it via a `contains_resource` guard.
- [ ] `main_opaque_pass_3d` renders without cloning `Core3dFrame`.
- [ ] `SystemState<P>` exists in moonfield-ecs with `new(world)` and `get(world)`; `RenderCommand<P>` declares `type Param: SystemParam` (read-only) and `fn render(world, item, pass, param)`; `DrawMesh` holds no state.
- [ ] `RenderPipelineCache` maps a (shader, state) key to `CachedRenderPipelineId`; `Opaque3d` exposes `cached_pipeline()`; `DrawMesh` binds `item.cached_pipeline()`.
- [ ] `TrackedRenderPass` deduplicates pipeline, vertex-buffer, index-buffer, and bind-group binds; draw functions receive `&mut TrackedRenderPass`.
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, and `python3 scripts/verify_agents.py` pass.

## Risks

- `RenderCommandState::draw` fetches `Param` from `&World` while holding `&mut self`; moonfield render systems already hold `&mut World`, so the borrow is feasible, but the per-phase write lock Bevy takes on `DrawFunctions` (because `draw` is `&mut self`) must be replicated, or the `&World` resource reads inside a draw conflict with the registry borrow.
- `SystemState<P>` is new public surface in moonfield-ecs; constraining `RenderCommand::Param` to read-only preserves the property that the render schedule does not mutate the render world from within a draw call.
- `RenderPipelineCache` is a new subsystem; a stale or missing cache key produces a wrong or absent pipeline, and without eviction the cache grows with distinct (shader, state) combinations.
- `TrackedRenderPass` deduplication is correct only while the cached state matches the live Vulkan state; `reset_tracking()` must run whenever a render pass begins or the command buffer is reset, or a skipped bind leaves stale state.
- Moving sort to `PhaseSort` requires every queue system to be ordered before it; a mis-ordered queue system silently produces unsorted rendering.
- Removing the `Core3dFrame` clone requires reordering how `main_opaque_pass_3d` borrows `WindowSurfaces` and the frame; an incorrect restructure trades the clone for a borrow-check failure or a stale-frame read.

## Alternatives considered

- **Stateless `RenderCommand` with tuple composition (Bevy's `SetItemPipeline` / `SetMeshBindGroup` / `DrawMesh` split)**: rejected for this proposal. Bevy's composition depends on a bind-group cache, a pipeline cache, and a multi-material bind-group system that moonfield's RHI does not have; splitting `DrawMesh` into `SetXxx` commands before that infrastructure exists is composition for its own sake. Revisit once a bind-group cache lands.
- **Sorted-merge batching and instancing**: conditional. Batching requires moving per-instance data out of push constants into an instance buffer, a larger RHI change, and pays off only when the same mesh is instanced many times. Deferred unless scenes reuse meshes across many entities; until then each item stays one `draw_indexed(.., 1, ..)`.
- **Binned phases with multi-draw indirect (Bevy's `BinnedRenderPhase`)**: rejected at this scale. Binning and GPU-culled multidraw pay off at full PBR scale with thousands of draws; at a few materials and phases they add bookkeeping (`BinKey` / `BatchSetKey` / three-bucket storage) without measurable benefit.
- **Bevy's `ViewQuery` / `ItemQuery` on `RenderCommand`**: rejected. moonfield's pass already sets viewport, depth, and cull state, and `DrawMesh` reads only the item; moonfield-ecs `Query` carries no per-entity cached state (`State = ()`), so view and item query parameters add concepts with no backing machinery. Only `Param` is retained.
- **A separate per-frame `prepare` optimization**: folded into `SystemState`. `SystemState::get` already removes the per-item resource fetches; the `prepare` hook stays as an empty default for future warmup, not as a standalone change.
- **Keeping the `HashMap` dispatch and per-item resource fetches**: rejected. The per-item hash lookup and triple `get_resource` are per-frame waste with no benefit over an indexed `Vec` and a cached param state.

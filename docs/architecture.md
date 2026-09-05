# Architecture

This page describes the runtime mechanisms of moonfield — the parts that code,
crate READMEs, and [Agent Notes](../.agents/notes/README.md) don't carry. Read it
before changing the ECS, winit, window, or editor layers. The composition and
ownership patterns are recorded in the linked Agent Notes where a decision was
made.

## Frame loop

The frame loop is **redraw-driven**: `about_to_wait` only
decides `ControlFlow` and requests redraws; the frame
(`moonfield_time::update_time` → `App::update` → `sync_windows` →
`App::render` → frame-state clearing → exit check) runs inside
`WindowEvent::RedrawRequested`. Update pacing is governed by the `WinitSettings`
resource (`focused_mode`/`unfocused_mode`: `UpdateMode::Continuous` or
`Reactive { wait, react_to_* }`; presets `game()` default / `desktop_app()` /
`continuous()`), re-read every frame decision so systems can change it at
runtime. External threads and UI toolkits wake an idle Reactive loop via the
`EventLoopProxyWrapper` resource (`wake_up()`, sends `WinitUserEvent::WakeUp`).

## ECS core

`moonfield-ecs` is a single-threaded, archetype-storage ECS in the style of a
mainstream retained-mode ECS. Systems declare their data access through system
params: `Res<T>`/`ResMut<T>` (resources), `Query<Q>` (archetype queries),
`Local<T>` (per-system state), `Commands` (deferred world mutations),
`MessageReader<M>`/`MessageWriter<M>` (buffered messages), plus
`Option<Res<T>>` and tuples up to 8; exclusive `FnMut(&mut World)` systems are
still supported. `Component` and `Resource` are blanket impls — no derive.

A `Schedule` groups systems under a `ScheduleLabel` and orders them with
`before`/`after` constraints (stable topological sort, single-threaded
executor). `Commands` queue into a world-global buffer that
`World::apply_commands` drains after **every** system run, so a system's
commands are visible to later systems in the same run; the world's change tick
advances once per schedule run. `App` owns separate main-world and render-world
schedule maps. Main-world execution includes
`Startup`/`First`/`Update`/`PreRender`/`Shutdown`; the render-world `Render`
schedule runs after extraction. `First` starts every update phase and owns the
message buffer swap. Exit is signaled by inserting the `AppExit` resource (e.g.
via `Commands::insert_resource`), not by a system return value. The low-level
archetype query trait is `WorldQuery`, distinct from the `Query<Q>` system
param.

Queries accept an optional second type parameter — the filter:
`Query<&Transform, With<MeshRenderer>>`, `Query<&mut Transform,
Without<ChildOf>>`, `Query<&T, Or<(With<A>, With<B>)>>` (tuples of filters
conjoin, `Or` disjoins, `()` is no filter). Filters are archetypal: each is
evaluated once per archetype against its component type set at iterator
construction, never per entity. The same filtering is available imperatively
via `World::query_filtered::<Q, F>()` / `query_filtered_mut`.

## Component hooks

Each component type can register `on_add`/`on_insert`/`on_discard`/`on_remove`
hooks imperatively via `World::register_component_hooks::<T>()` (plus
`on_despawn`, fired before discards with every component still in place).
Discard hooks fire *before* the structural change, so the old value is still
readable; add/insert/remove fire *after*. Hooks therefore always see a
structurally consistent world and get full `&mut World` access — that is the
seam relationships build on. A hook is taken out of the registry while
running: no same-hook recursion, while nested chains across different
components fire normally. Known gaps: `spawn_batch` and `World::clear` don't
fire hooks.

## Relationships and hierarchy

Generic `Relationship`/`RelationshipTarget` traits in `moonfield-ecs` are kept
in sync by component hooks and registered per type via
`World::register_relationship::<R>()`; `ChildOf`/`Children` sit on top via
`World::register_hierarchy()`. Semantics: inserting `ChildOf` links the child
into the parent's auto-created `Children`; remove/replace unlinks (an emptied
`Children` is dropped); despawning a parent despawns children recursively
(`Children::LINKED_SPAWN`); a `ChildOf` insert that would close a cycle
**panics** on an ancestor-chain walk — a check the reference implementation
deliberately doesn't do.

`Transform`/`GlobalTransform` live in `moonfield-math` as plain math types
with no ECS knowledge (`moonfield-ecs` already depends on it, keeping the
dependency directions acyclic). `ensure_global_transforms` and
`propagate_transforms` run as normal param systems in `Update` and `PreRender`,
wired by `moonfield_app::HierarchyPlugin`. A `PreRender` system that mutates
`Transform` must run before `ensure_global_transforms`; the editor applies this
ordering so camera extraction reads the same frame's `GlobalTransform`.

## Messages

`moonfield-ecs::message` is the buffered-event channel (the reference
implementation's current dev branch calls these *messages* rather than
*events*). `App::add_message::<M>()` inserts the `Messages<M>` resource and
registers the type in the `MessageRegistry` resource; `message_update_system`
runs in `First` and swaps each registered store's double buffer once per
frame, giving every message a two-frame lifetime. Writers use the
`MessageWriter<M>` param; readers use `MessageReader<M>`, whose per-system
cursor (`MessageCursor<M>`, held as the param's persistent state) tracks
which messages that system has seen — each reader consumes each message
exactly once. Exclusive systems and non-system consumers (the editor's
render loop) hold a `MessageCursor` directly. The windowing backend's
lifecycle events (`WindowEventKind`) and raw winit events travel on this
channel; `InputState` stays latched state with its own frame-scoped clearing.

## Time

`moonfield-time` provides the `Time<Real>` / `Time<Virtual>` / `Time<Fixed>` /
generic `Time` clock resources: delta and elapsed as `Duration` plus f32/f64
seconds, wrapped elapsed, and on the virtual clock pause, relative speed, and
a `max_delta` clamp. `TimePlugin` (in `moonfield-app`, next to
`HierarchyPlugin` — the app crate depends on the time crate so `App::update`
can drive the fixed loop) inserts the resources; the winit backend advances
them via `moonfield_time::update_time` once per frame at frame start, before
`App::update`, lazily inserting missing clocks so the editor path works
without the plugin. `Timer`/`Stopwatch` are not ported.

## Fixed update

`App::update` runs `First`, then the fixed-timestep loop, then `Update`. The
loop (`moonfield_time::run_fixed_main_schedule`) accumulates the virtual delta
into `Time<Fixed>`'s overstep and, once per full `timestep()` (default 64
Hz), runs `FixedFirst` → `FixedPreUpdate` → `FixedUpdate` → `FixedPostUpdate`
→ `FixedLast` (plus anything registered directly under the `FixedMain`
umbrella) — so fixed schedules run 0, 1, or N times per frame. During each
iteration the generic `Time` resource mirrors `Time<Fixed>` (delta ==
timestep); afterwards it is restored to virtual time. Without `TimePlugin`
there is no `Time<Fixed>` and the loop is a no-op. The winit backend does no
fixed-step-specific input latching for now.

## Windows are ECS entities

The backend spawns the primary window entity in `resumed` (adopting a
pre-spawned `Window` entity if user code created one at startup) and
attaches `Window` + `PrimaryWindow` + `RawHandleWrapper` + `CachedWindow`
components. winit→ECS direction (resize/DPI/focus) writes back into the `Window`
component immediately in `window_event`; ECS→winit direction (title/cursor_mode
mutations by gameplay/editor code) is applied once per frame after `App::update`
by `sync_windows`, which diffs the live component against the `CachedWindow`
cache (a per-field cached-window diff, without change detection). `WinitWindows`
(resource) holds the `Entity ↔ WindowId` mapping. There is no `WindowRequests`
channel — mutate the component.

Window lifecycle events (`close_requested`/`resized`/`focus_*`/
`scale_factor_changed`) travel on the message channel — the
`Messages<WindowEventKind>` resource, written by the backend as events arrive
and read with per-reader cursors (see Messages); every entry carries the
window `Entity` (multi-window-shaped, single-window today). Exit policy
mirrors the `auto_accept_quit` convention:
`CloseRequested` exits immediately by default; a caller sets
`WindowControl::set_auto_exit_on_close(false)` to take over and later
`WindowControl::request_exit()`.

## Input flow

`moonfield-winit` translates winit events into the `InputState` world resource
(frame-latched; cleared each frame after the update) for ECS systems to read
during the update. Keys/buttons are strongly typed (`KeyCode`/`MouseButton`
mirror enums in `moonfield-window`, converted 1:1 in
`moonfield-winit::converters`); auto-repeat presses arrive flagged `repeat` and
never re-arm `just_pressed`; modifier state is available both as ordinary key
presses and via the `Modifiers` bitflags convenience; scroll deltas keep their
original `MouseScrollUnit` (no px→line folding; convert with
`MOUSE_SCROLL_PIXELS_PER_LINE` = 100). `just_pressed`/`just_released` edges are
frame-scoped: cleared by `InputState::end_frame` once per frame after the
update has consumed them. Input is not latched separately for fixed steps:
fixed systems read the same per-frame `InputState`.

## Renderer and editor composition

The renderer is Vulkan-only through `ash` and always available in the default
build. The engine-level clip convention is Y-up with reverse-Z, with any Vulkan
viewport adjustment handled at the Vulkan boundary.

`App::render` runs `PreRender` in the main world, rebuilds render snapshot
entities through handwritten extraction, then runs `RenderPrepare`,
`RenderQueue`, and `Render` in the render world. `RenderPrepare` updates
persistent GPU data from the snapshot, `RenderQueue` builds per-frame view and
phase work, and `Render` records and submits commands. Snapshot entities carry
`MainEntity`; render-world resources persist across the rebuild and own
cross-frame caches and GPU objects. The editor's
orbit-camera update runs before the `PreRender` hierarchy propagation pass, so
camera extraction receives the updated global pose without a frame of latency.

`RenderPlugin` (in `moonfield-render-core`, Selene) creates the Vulkan instance
and logical device at build time and inserts the `RenderDevice` resource
(Lunar Mare, `moonfield-rhi`) only in the render world. It is
`Arc`-cloneable and headless-tolerant: without a driver the plugin logs an error
and inserts nothing. Per-window GPU state (surface, swapchain, command buffers,
frame synchronization) lives in the render-world `WindowSurfaces` resource
keyed by `MainEntity`; `RenderPlugin` also registers the window frame-loop
systems — `create_window_surfaces` (`RenderPrepare`), `acquire_window_frames`
and `submit_window_frames` (`Render`, the ordering anchors pass systems chain
against). Acquire is gated by the `WindowFrameDemand` resource that extraction
writes, so a window frame only exists when a consumer has content to present.

The editor is a library crate providing `EditorPlugin`, a regular plugin
composing the engine crates. `EditorPlugin` does **not** own the event
loop or the window — it layers on top of `WinitPlugin` (which must be added
first), reading window events and editor resources in `PreRender`.
`EditorMainState` owns egui input, docks, selection, asset/scene actions,
camera controls, and gizmos, and stages the frame as a `PendingEditorFrame`
resource. `extract_editor_frame` moves it into the render world (merging into
an unconsumed frame so texture deltas are never dropped), reports the viewport
panel size through `RenderTargetSizes`, and sets `WindowFrameDemand`. The egui
backend is data, not an object: `EguiPipeline`, `EguiTextures`, and
`EguiFrameResources` resources driven by the `prepare_egui_frame` → `egui_pass`
→ `editor_frame_done` `Render` systems. Render feedback (viewport texture id,
presented-frame count) returns through the `EditorFeedbackChannel` cloned into
both worlds.

One deliberate exception to backend abstraction: the **editor binds winit
directly** — it holds an `Arc<winit::window::Window>`, feeds
`winit::event::WindowEvent`s into `egui_winit::State`, and reads the raw-event
message channel. The `moonfield-window`/`moonfield-winit` split keeps the
*render* path backend-agnostic (render-core only sees `RawHandleWrapper`), not
the editor; swapping windowing backends means rewriting the editor's egui glue.

`moonfield-camera` owns the scene-facing `Camera`, `PrimaryCamera`,
`CameraTarget`, `RenderTarget`, projection, and view math without depending on
the Vulkan RHI. Camera extraction in `moonfield-render-core` produces `ExtractedView`
from `Camera` + `GlobalTransform` + `MainEntity`; an optional `CameraTarget`
selects the primary window or editor viewport without changing serialized
camera fields. `RenderFeaturePlugin` prepares revision-matched GPU meshes in
`RenderPrepare` and rebuilds `Core3dFrame` in `RenderQueue` every render tick.
Each `Core3dView`
owns a front-to-back `RenderPhase<Opaque3d>`; the mesh feature's
`queue_opaque_3d` fills it with live-mesh items and registers `DrawMesh` in the
phase's `DrawFunctions` registry, and the pass dispatches items to their
registered draw functions.

The `main_opaque_pass_3d` system consumes the primary view targeting the editor
viewport and records it into the persistent `OffscreenTarget` held by the
`ViewTargets` resource (final layout `SHADER_READ_ONLY_OPTIMAL`) sampled by
egui. Referenced mesh assets are copied
into `ExtractedMeshes`; GPU buffers in the render-world `PreparedGpuMeshes`
resource are reused only when their `AssetRevision` matches. The pass
only consumes prepared buffers while recording. Per-draw data (mvp + flat
color) is a `DrawData` record carved from the render-world `FrameDrawArena`;
the draw pushes a single `GpuPtr` to it through `push_data`. The offscreen
target carries a depth attachment
(`OffscreenTarget::new_with_depth`; reverse-Z — depth clears to 0.0 and the
compare op is `GREATER_OR_EQUAL`), so overlapping meshes occlude.
Slang packs matrices row-major by default while glam's `to_cols_array()` is
column-major, so the matrix ships column-major inside `DrawData` and the
shader declares `column_major float4x4 mvp;`.
The Hierarchy dock panel lists the entity tree (from `ChildOf`/`Children`,
labeled by `Name`) and selects an entity; the Inspector panel renders
auto-generated editing UI for the selected entity's registered components.
Setting `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` signals exit via the shared
`WindowControl` after N rendered frames, which allows automated smoke tests of
startup/shutdown on a machine with a display.

The game (`WinitPlugin`) and editor paths share the same `WinitPlugin` runner
and the `App::run()` → `Runner` architecture.

## Reflection and the inspector

`moonfield-reflect` is a deliberately small reflection layer: the `Reflect`
trait enumerates named fields (`FieldInfo`), exposes dynamic `field`/`field_mut`
access as `&dyn Reflect`, and downcasts leaves via `Any`; leaf impls cover the
scalar types, `String`, and the math layer's vector/quaternion types.
`#[derive(Reflect)]` lives in `moonfield-reflect-derive` — the one sanctioned
proc-macro crate (named-field non-generic structs only, `#[reflect(ignore)]`
to skip a field). There is no `DynamicStruct`, type registry, or
serialization.

The editor's Inspector is fully generic over it: `EditorPlugin` inserts an
`InspectorRegistry` world resource (`register::<T: Component + Reflect>()`),
and each registered component on the selected entity renders as a collapsing
header via the `reflect_ui` walker — nested structs recurse, leaf widgets
dispatch by downcast (quaternions edit as Euler-XYZ degrees).

## Assets

`moonfield-asset` is a zero-dependency, synchronous asset layer. `Assets<T>`
is a slot-map store held as one world resource per asset type
(add/get/get_mut/remove/iter; freed slots are reused with a bumped generation
so stale handles resolve to `None`), and `Handle<T>` is an index+generation
handle that is `Copy` regardless of `T` and a component/resource through the
blanket impls.

Every insertion and mutable access assigns a monotonic `AssetRevision`.
Render-world asset snapshots use `(AssetId, AssetRevision)` to detect changed
payloads while preserving prepared GPU data for unchanged assets. Calling
`get_mut` advances the revision conservatively even when the returned value is
left unchanged.

On top of the stores sits `AssetServer`, also a world resource: an
extension-dispatching, path-caching loader. `AssetLoader` implementors declare
the extensions they handle and return the payload type-erased (`Send + Sync`,
because the blanket `Resource` impl requires it); `AssetServer::load` picks a
loader by the path's extension, downcasts the payload to the requested `T`,
and inserts it into the caller's `Assets<T>`. The cache keys on `(TypeId,
PathBuf)`, so a path loads at most once per asset type and the same file as
two types stays distinct; a cached id that no longer resolves (the asset was
removed) triggers a reload. Loading happens on the calling thread — there is
no task pool, no async, no hot reload.

Two real source asset types sit on these stores in `moonfield-render-feature`.
`MeshHandle` and `SplatCloudHandle` are plain newtypes that become components
through the ECS blanket implementation. `Mesh` (`moonfield-render-feature::mesh`) is
merged triangle geometry with a precomputed AABB and the source path, and `SplatCloud`
(`moonfield-render-feature::splat::cloud`) wraps the Gaussian scene the same way.

glTF 2.0 (`.gltf`/`.glb`) is the sole asset source format, parsed with the
`gltf` crate. Mesh import merges every TRIANGLE primitive in the file into
one positions + indices pair (vertex offsets applied; non-indexed primitives
get sequential indices) and drops POINTS primitives, node transforms, and
materials — known slices: no per-primitive split and no materials, so a file
imports as one flat-colored mesh. Splat import reads the Khronos
`KHR_gaussian_splatting` extension: a POINTS primitive carrying
`KHR_gaussian_splatting:*` attributes, float component types only (the
quantized int variants are rejected), kernel `"ellipse"` only, no SPZ
compression sub-extensions. The loader converts the glTF render-space values
into the training-space conventions `GaussianScene` keeps: scale → ln,
opacity → logit, quaternion xyzw → wxyz, degree-0 SH verbatim into `f_dc`
(the `0.282·c + 0.5` bias is a shading-time op, never stored), and
higher-degree SH — one RGB VEC3 per coefficient — transposed into the
channel-blocked `f_rest` layout (coefficient `c = l*l − 1 + n` of channel
`ch` lands at `f_rest[ch * 15 + c]`), zero-filling missing degrees. Because
gltf-json maps the unknown extension semantics to `Checked::Invalid`, the
splat loader parses without validation and reads the attribute map from the
raw JSON, while mesh loading uses validated `gltf::import`. The PLY loader is
removed; training-side interop will be served by a
`KHR_gaussian_splatting` exporter.

The editor's `GltfLoader` dispatches on content: it sniffs the file bytes for
The editor's `GltfLoader` produces `Mesh` assets by default. With the
`splat` Cargo feature, it also dispatches `KHR_gaussian_splatting` files to
`SplatCloud` assets. The Hierarchy panel loads assets through a path field +
Load button routed through the `AssetServer` (loading the same file twice
reuses the asset slot), and the loaded entity appears in the tree named
after the file — mesh entities carry `MeshRenderer` in `DEFAULT_MESH_COLOR`.
Training/optimizer state stays outside the `World`.

## Scenes and templates

Scene save/load is a synchronous miniature of the reference implementation's
0.20 template pipeline, split across `moonfield-ecs` and `moonfield-scene`;
there is no runtime reflection anywhere in it. `moonfield-ecs` holds the
typed half: a `Template` is plain data that builds its `Output` inside a
`TemplateContext { world: &mut World }`, and every `Clone` type is its own
template (building clones). `moonfield-scene` adds `HandleTemplate<T>` — a
path to load through the `AssetServer`, or an already-resolved handle — and
the two-phase scene form: a `ResolvedScene` bundles one entity's type-erased
templates (`SceneTemplate`, blanket-implemented for every `Template` with
`Component` output) plus its children's resolved scenes, and `apply` spawns
the subtree, linking children with `ChildOf` (the world must have called
`register_hierarchy`). Building is immediate, on the calling thread; there is
no async queue.

The text carrier is glTF 2.0 JSON (`.gltf`, via `gltf-json`), written and
read by `save_scene`/`load_scene` (plus `_to_file`/`_from_file` helpers). The
node tree carries the hierarchy, node TRS fields carry `Transform` (glTF is
Y-up right-handed like the engine, so values cross verbatim), and perspective
cameras ride the root `cameras` array. Which components participate is
decided by a `SceneRegistry` world resource under stable short names
(`"transform"`, `"mesh_renderer"`, `"splat_cloud"` — never a Rust type path,
so renames don't break files). Entries come in two kinds: native mappings
(transform, camera, hierarchy) that read and write node fields directly, and
extras-channel entries that serialize into `node.extras.components.<name>` —
generic for `Clone + Serialize + DeserializeOwned` components, custom
save/load hooks for cases like `Name` (routed to `node.name`) and path-backed
handles (save resolves the handle to its source path; load builds a
`HandleTemplate::Path`, so scene load resolves assets through the
`AssetServer` cache). Savable roots are entities with at least one registered
component and no `ChildOf`; unregistered components and component-less
subtrees are skipped, `GlobalTransform` is never registered (propagation
recomputes it after load), and unknown extras keys skip rather than error, so
a scene written by a newer registry still loads.

Versus the vendored 0.20-dev source, deliberately skipped: the `bsn!`
proc-macro DSL, `ScenePatch` caching, `QueuedScenes`/`WaitingScenes`
(async-only), the `BundleWriter` bump arena, and named entity references; on
the glTF side the scene document uses only the node/camera scaffold —
asset-level mesh import lives in `moonfield-render-feature` (see Assets), and
material import is untouched.

The editor wires both halves: `EditorPlugin` inserts `editor_asset_server()`
(the content-sniffing `GltfLoader`) and `editor_scene_registry()` (native
transform/camera/hierarchy, `Name` on `node.name`, and `mesh_renderer` /
`splat_cloud` as path-string custom entries — save writes the asset's source
path, load rebuilds the component through the `AssetServer` cache, and a
scene-loaded `MeshRenderer` gets `DEFAULT_MESH_COLOR`) as world resources,
and the Hierarchy panel carries a Scene path field with Save/Load buttons
(`SceneIoState`).

## Threading model

The winit event loop and all Vulkan objects live on the **main thread**; nothing
is `Send` across threads yet. Render work and ECS `World` access are confined to
that thread. Once multi-threaded rendering lands, gameplay code must hand off to
a render thread via a command queue (the logic thread produces render commands,
the render thread owns all Vulkan objects); GPU/native objects are never shared
directly across threads.

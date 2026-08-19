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
advances once per schedule run. `App` drives five schedules —
`Startup`/`First`/`Update`/`Render`/`Shutdown` (`First` runs at the start of
every update phase; the message buffer swap lives there). Exit is signaled by
inserting the `AppExit` resource (e.g. via `Commands::insert_resource`), not
by a system return value. The low-level archetype query trait is `WorldQuery`,
distinct from the `Query<Q>` system param.

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
`propagate_transforms` run as normal param systems in `Update`, wired by
`moonfield_app::HierarchyPlugin`.

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

`moonfield-time` provides the `Time<Real>` / `Time<Virtual>` / generic `Time`
clock resources: delta and elapsed as `Duration` plus f32/f64 seconds, wrapped
elapsed, and on the virtual clock pause, relative speed, and a `max_delta`
clamp. `TimePlugin` inserts the resources; the winit backend advances them via
`moonfield_time::update_time` once per frame at frame start, before
`App::update`, lazily inserting missing clocks so the editor path works
without the plugin. `Time<Fixed>` is deferred together with the fixed-update
schedule; `Timer`/`Stopwatch` are not ported.

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
update has consumed them. There is no fixed-update schedule yet.

## Renderer and editor composition

The renderer is Vulkan-only through `ash` and always available in the default
build. The engine-level clip convention is Y-up with reverse-Z, with any Vulkan
viewport adjustment handled at the Vulkan boundary.

The device-level Vulkan singletons are shared: `RenderPlugin` creates the
instance and logical device at build time and inserts them as the
`RenderDevice` world resource (`Arc`-cloneable; headless-tolerant — with no
driver it logs an error and inserts nothing, never panics). `WindowRenderer`
borrows the shared device and owns only the window-bound objects (surface,
swapchain, framebuffers, frame sync); presentation support is validated per
window.

The editor is a library crate providing `EditorPlugin`, a regular plugin
composing the engine crates. `EditorPlugin` does **not** own the event
loop or the window — it layers on top of `WinitPlugin` (which must be added
first), reading the `WinitWindow`/`InputState`/`WindowControl` resources and
the raw-event message channel the backend registers, and lazily building the `WindowRenderer` +
egui state (`EditorState` keeps only editor-only state) on the first render
tick. Render-phase systems register via `App::add_systems(Render, ...)`; the
winit backend calls `App::render` every frame after `App::update`, which drives
the editor to build the egui UI and record it (plus the viewport scene) into
the same swapchain.

The viewport renders the ECS scene, queried straight from the `World`
(single-threaded render seam — no extract/snapshot layer), into an
`OffscreenTarget` (final layout `SHADER_READ_ONLY_OPTIMAL`) that egui samples
as a user texture in the Viewport dock panel. The scene components live in
`moonfield-render::scene`: the `Camera` + `PrimaryCamera` + `GlobalTransform`
entity provides view/projection (aspect follows the panel size), and every
`MeshRenderer` + `GlobalTransform` entity is drawn as a colored unit cube via
push constants (no depth attachment yet — overlapping cubes don't occlude).
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
blanket impls. Loading is synchronous by the caller; there is no async
`AssetServer` or task pool yet.

`SplatCloud` is the first real asset: plain data in
`moonfield-renderer::splat::cloud` (wraps the Gaussian scene plus source path
and a precomputed AABB; the crate stays ECS-free, so `SplatCloudHandle` is a
plain newtype that is a component through the blanket impl). The editor's
Hierarchy panel loads PLY files through a path field + Load button, and the
loaded entity appears in the tree named after the file. Viewport rendering of
splats is a placeholder — the cloud's AABB drawn as a fixed-color box through
the existing cube pipeline — until real splat rasterization lands.
Training/optimizer state stays outside the `World`.

## Threading model

The winit event loop and all Vulkan objects live on the **main thread**; nothing
is `Send` across threads yet. Render work and ECS `World` access are confined to
that thread. Once multi-threaded rendering lands, gameplay code must hand off to
a render thread via a command queue (the logic thread produces render commands,
the render thread owns all Vulkan objects); GPU/native objects are never shared
directly across threads.

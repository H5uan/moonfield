# Architecture

This page describes the runtime mechanisms of moonfield — the parts that code,
crate READMEs, and [Agent Notes](../.agents/notes/README.md) don't carry. Read it
before changing the winit, window, or editor layers. The composition and ownership
patterns are recorded in the linked Agent Notes where a decision was made.

## Frame loop

The frame loop is **redraw-driven**: `about_to_wait` only
decides `ControlFlow` and requests redraws; the frame (`App::update` →
`sync_windows` → `App::render` → frame-state clearing → exit check) runs inside
`WindowEvent::RedrawRequested`. Update pacing is governed by the `WinitSettings`
resource (`focused_mode`/`unfocused_mode`: `UpdateMode::Continuous` or
`Reactive { wait, react_to_* }`; presets `game()` default / `desktop_app()` /
`continuous()`), re-read every frame decision so systems can change it at
runtime. External threads and UI toolkits wake an idle Reactive loop via the
`EventLoopProxyWrapper` resource (`wake_up()`, sends `WinitUserEvent::WakeUp`).

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
`scale_factor_changed`) travel on a separate channel — the `WindowEvents` world
resource; every entry carries the window `Entity` (multi-window-shaped,
single-window today). Exit policy mirrors the `auto_accept_quit` convention:
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
`MOUSE_SCROLL_PIXELS_PER_LINE` = 100). `just_pressed` is frame-scoped in
`on_update` and fixed-step-scoped in `on_fixed_update` (delivered to exactly one
step, never lost across frames).

## Renderer and editor composition

The renderer is Vulkan-only through `ash` and always available in the default
build. The engine-level clip convention is Y-up with reverse-Z, with any Vulkan
viewport adjustment handled at the Vulkan boundary.

The editor is a library crate providing `EditorPlugin`, a regular plugin
composing the engine crates. `EditorPlugin` does **not** own the event
loop or the window — it layers on top of `WinitPlugin` (which must be added
first), reading the `WinitWindow`/`InputState`/`WindowControl`/`RawWindowEvents`
resources the backend registers, and lazily building the windowed Vulkan
renderer (`WindowRenderer`) + egui state on the first render tick. The editor
registers a render-phase system via `App::add_render_system`; the winit backend
calls `App::render` every frame after `App::update`, which drives the editor to
build the egui UI and record it (plus the viewport scene) into the same
swapchain. The scene renders into an `OffscreenTarget` (final layout
`SHADER_READ_ONLY_OPTIMAL`) that egui samples as a user texture in the Viewport
dock panel. Setting `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` signals exit via the
shared `WindowControl` after N rendered frames, which allows automated smoke
tests of startup/shutdown on a machine with a display.

The game (`WinitPlugin`) and editor paths share the same `WinitPlugin` runner
and the `App::run()` → `Runner` architecture.

## Threading model

The winit event loop and all Vulkan objects live on the **main thread**; nothing
is `Send` across threads yet. Render work and ECS `World` access are confined to
that thread. Once multi-threaded rendering lands, gameplay code must hand off to
a render thread via a command queue (the logic thread produces render commands,
the render thread owns all Vulkan objects); GPU/native objects are never shared
directly across threads.
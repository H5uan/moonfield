# Repository Guidelines

Welcome to **moonfield** — a Rust workspace implementing a Vulkan RHI, a Bevy-style ECS/app framework, and an editor. This guide helps you get oriented quickly.

## Project Structure & Module Organization

The workspace is a Cargo-managed monorepo under `crates/`:

```
crates/
  moonfield/          # Binary crate — the main executable entry point (src/main.rs)
  moonfield-app/      # Bevy-style App/Plugin framework (Plugin, PluginGroup, App, Resources)
  moonfield-asset/    # Sync-only Assets<T> store + Handle<T> (index+generation); no deps, no async
  moonfield-base/     # Shared base types and utilities
  moonfield-editor/   # Editor plugin (library crate, egui + egui_dock + egui-ash-renderer): dock panels, offscreen viewport
  moonfield-math/     # glam re-export + domain types (Dir3/Ray3d), Transform/GlobalTransform,
                      # engine clip-space conventions — the workspace math single entry (bevy_math pattern)
  moonfield-reflect/  # Mini Reflect: Reflect trait (named fields, dynamic read/write, nesting) + leaf
                      # impls; NOT bevy_reflect. Depends on glam directly (avoids a math↔reflect cycle)
  moonfield-reflect-derive/ # #[derive(Reflect)] proc-macro (the one sanctioned proc-macro crate)
  moonfield-render/   # Lunar Mare — Vulkan-only rendering RHI (ash, src/vulkan/); public resource descriptions
                      # (Format, BufferUsage, VertexBufferLayout) in src/types.rs; swapchain, shaders,
                      # offscreen targets (offscreen.rs), windowed frame loop (window_target.rs),
                      # shared RenderDevice resource + scene components (Camera/PrimaryCamera/MeshRenderer)
  moonfield-renderer/ # Lunaris — scene rendering & algorithms (splat/rt/gi), RenderAlgorithm phase abstraction,
                      # 3DGS training (splat::train); targets the Vulkan RHI and re-exports it as `rhi`
  moonfield-time/     # Time<Real>/Time<Virtual>/Time clocks + TimePlugin (bevy_time port);
                      # the winit backend advances them once per frame before App::update
  moonfield-window/   # Abstract windowing types (Window/PrimaryWindow/RawHandleWrapper components,
                      # KeyCode/MouseButton/Modifiers mirror enums, InputState/InputEvent,
                      # WindowEvents/WindowControl resources), no backend deps
  moonfield-winit/    # Windowing backend (winit), bridges winit Window → moonfield-window
                      # components (WinitWindows Entity↔WindowId map, CachedWindow field-diff sync)
```

The editor is a library crate that provides [`EditorPlugin`], a regular Bevy-style plugin composing the engine crates. `EditorPlugin` does **not** own the event loop or the window — it layers on top of `WinitPlugin` (which must be added first), reading the `WinitWindow`/`InputState`/`WindowControl`/`RawWindowEvents` resources the backend registers, and lazily building the windowed Vulkan renderer (`WindowRenderer`) + egui state on the first render tick. The device-level Vulkan singletons are shared: `RenderPlugin` creates the instance + logical device once and inserts them as the `RenderDevice` world resource; `WindowRenderer` borrows them via `Arc`s and owns only the window-bound objects (surface, swapchain, framebuffers, frame sync). The editor registers a render-phase system via `App::add_systems(Render, ...)`; the winit backend calls `App::render` every frame after `App::update`, which drives the editor to build the egui UI and record it (plus the viewport scene) into the same swapchain. This mirrors how `bevy_egui` layers on `bevy_winit` rather than replacing it. The ECS scene is queried straight from the `World` (single-threaded render seam) and rendered into an `OffscreenTarget` (final layout `SHADER_READ_ONLY_OPTIMAL`) that egui samples as a user texture in the Viewport dock panel: the `Camera` + `PrimaryCamera` + `GlobalTransform` entity provides view/projection (aspect follows the panel size), and every `MeshRenderer` + `GlobalTransform` entity is drawn as a colored unit cube via push constants (no depth attachment yet — overlapping cubes don't occlude). The Hierarchy dock panel lists the entity tree (`ChildOf`/`Children`, `Name` labels) and selects an entity; the Inspector panel renders auto-generated editing UI for every `InspectorRegistry`-registered component on the selected entity via `moonfield_reflect` (Transform/Camera/MeshRenderer registered by `EditorPlugin`; Quat fields edit as Euler-XYZ degrees), with edits propagating through `HierarchyPlugin`'s propagation into the viewport. The egui stack is anchored to egui-ash-renderer's compatibility table (currently egui 0.33 / egui-winit 0.33 / egui-ash-renderer 0.11 + gpu-allocator / egui_dock 0.18, ash 0.38, winit 0.30) — bump them together. Setting `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` signals exit via the shared `WindowControl` after N rendered frames, which allows automated smoke tests of startup/shutdown on a machine with a display (`MOONFIELD_EDITOR_AUTO_CLOSE=5 cargo run --example editor -p moonfield-editor`). To use the editor, add `RenderPlugin` (creates the shared `RenderDevice`), `WinitPlugin` (with `WinitSettings::continuous()` for continuous redraw), and `HierarchyPlugin` (transform propagation into `GlobalTransform`) first, then `EditorPlugin`, then `app.run()`. The game (`WinitPlugin`) and editor paths share the same `WinitPlugin` runner and the `App::run()` -> `Runner` architecture.

Windows are ECS entities: the backend spawns the primary window entity in `resumed` (adopting a pre-spawned `Window` entity if user code created one at startup, Bevy-style) and attaches `Window` + `PrimaryWindow` + `RawHandleWrapper` + `CachedWindow` components. winit→ECS direction (resize/DPI/focus) writes back into the `Window` component immediately in `window_event`; ECS→winit direction (title/cursor_mode mutations by gameplay/editor code) is applied once per frame after `App::update` by `sync_windows`, which diffs the live component against the `CachedWindow` cache (Bevy's `changed_windows` pattern without change detection). `WinitWindows` (resource) holds the `Entity ↔ WindowId` mapping. There is no `WindowRequests` channel — mutate the component.

Input flows: `moonfield-winit` translates winit events into the `InputState` world resource (frame-latched; cleared each frame after the update) for ECS systems to read during the update. Keys/buttons are strongly typed (`KeyCode`/`MouseButton` mirror enums in `moonfield-window`, converted 1:1 in `moonfield-winit::converters`); auto-repeat presses arrive flagged `repeat` and never re-arm `just_pressed`; modifier state is available both as ordinary key presses and via the `Modifiers` bitflags convenience; scroll deltas keep their original `MouseScrollUnit` (no px→line folding; convert with `MOUSE_SCROLL_PIXELS_PER_LINE` = 100). `just_pressed`/`just_released` edges are frame-scoped: cleared by `InputState::end_frame` once per frame after the app update has consumed them. (There is no fixed-update schedule yet — see the roadmap below.)

Window lifecycle events (`close_requested`/`resized`/`focus_*`/`scale_factor_changed`) travel on a separate channel — the `WindowEvents` world resource; every entry carries the window `Entity` (multi-window-shaped, single-window today). Exit policy mirrors Godot's `auto_accept_quit`: `CloseRequested` exits immediately by default; a caller sets `WindowControl::set_auto_exit_on_close(false)` to take over and later `WindowControl::request_exit()` (signals via the shared `WindowControl`).

The frame loop is **redraw-driven** (bevy_winit's model): `about_to_wait` only decides `ControlFlow` and requests redraws; the frame (`moonfield_time::update_time` → `App::update` → `sync_windows` → `App::render` → frame-state clearing → exit check) runs inside `WindowEvent::RedrawRequested`. Update pacing is governed by the `WinitSettings` resource (`focused_mode`/`unfocused_mode`: `UpdateMode::Continuous` or `Reactive { wait, react_to_* }`; presets `game()` default / `desktop_app()` / `continuous()`), re-read every frame decision so systems can change it at runtime. External threads and UI toolkits wake an idle Reactive loop via the `EventLoopProxyWrapper` resource (`wake_up()`, sends `WinitUserEvent::WakeUp`).

## Build, Test, and Development Commands

| Command | Description |
|---|---|
| `cargo build` | Compile all workspace crates. |
| `cargo run` | Build and run the `moonfield` binary. |
| `cargo test` | Run all unit and integration tests across the workspace. |
| `cargo clippy` | Lint the codebase with Clippy. |
| `cargo fmt` | Format all Rust source files. |

The rendering crates support Vulkan only through `ash`. The renderer is always available in the default build. The engine-level clip convention is Y-up with reverse-Z, with any Vulkan viewport adjustment handled at the Vulkan boundary.

| Command | Description |
|---|---|
| `cargo check -p moonfield-render` | Check the Vulkan RHI. |
| `cargo check -p moonfield-renderer --features "splat,rt,gi"` | Check scene rendering + all algorithms on Vulkan. |
| `cargo test -p moonfield-render --test headless_triangle` | Run the Vulkan headless smoke test. |

Shader authoring: runtime Slang→SPIR-V compilation is provided by the Vulkan backend (`vulkan/shader.rs`). `ShaderModule::from_spirv` loads Vulkan SPIR-V bytecode directly; one offline Slang compile (`slangc -target spirv`) can also produce embedded shader bytes with `include_bytes!`.

`WindowRenderer`/`Swapchain`, `RenderPlugin` (owns the shared `RenderDevice` instance/device world resource), and `moonfield-editor` are all part of the Vulkan desktop path.

External native dependencies:

- **Slang** — `shader-slang-sys` (via `moonfield-render`) links the Slang compiler dynamically. Set `SLANG_DIR` (a prebuilt [Slang release](https://github.com/shader-slang/slang/releases) with `include/`, `lib/`, `bin/`) or install a recent Vulkan SDK (`VULKAN_SDK` is used as a fallback). The Slang shared library must also be on the runtime library path (`PATH` on Windows, `LD_LIBRARY_PATH` on Linux, `DYLD_LIBRARY_PATH` on macOS) when running binaries/tests.
- **libclang** — required by `bindgen` (used by `shader-slang-sys`).

## Continuous Integration

GitHub Actions (`.github/workflows/ci.yml`) runs on pushes to `master` and on PRs, across `ubuntu-latest`, `windows-latest`, and `macos-latest` (Apple Silicon):

- `rustfmt` — `cargo fmt --all -- --check`.
- `clippy` — `cargo clippy --workspace --all-targets -- -D warnings` on all three platforms.
- `test` — `cargo test --workspace` on all three platforms.
- `vulkan-smoke` — headless Vulkan triangle test on Ubuntu with lavapipe, alongside the workspace checks on Linux, Windows, and macOS.

The `.github/actions/setup-slang` composite action downloads a pinned Slang release and exports `SLANG_DIR` plus the runtime library path. On Linux, CI installs `mesa-vulkan-drivers` (lavapipe) so GPU-dependent tests (`headless_triangle`) run for real; on Windows/macOS they skip gracefully when no Vulkan driver is present.

## Threading Model

- The winit event loop and all Vulkan objects live on the **main thread**; nothing is `Send` across threads yet. Render work and ECS `World` access are confined to that thread.
- Once multi-threaded rendering lands, gameplay code must hand off to a render thread via a command queue (the logic thread produces render commands, the render thread owns all Vulkan objects); GPU/native objects are never shared directly across threads.

## Roadmap: ECS-Driven Infrastructure

Agreed direction (2026-08): drive all engine infrastructure through ECS, Bevy-style — dual goal of (a) architectural parity with Bevy ("our own Bevy") and (b) making everything inspectable/editable by the editor. Scope is the **middle layer**: the game-visible surface (scene entities, Transform hierarchy, camera, time) + an asset system + ECS mirrors of render resources. Explicitly **out of scope for now**: a separate render world / extract split (that belongs to the future multi-threaded rendering effort).

**Method:** port runtime mechanisms from the local Bevy checkout (`target/bevy-src`, 0.20.0-dev), architecture-level borrowing rather than API mirroring. No proc-macro crates with one sanctioned exception: `moonfield-reflect-derive` (mini Reflect, milestone 7). `Component`/`Resource` stay blanket impls, system params use hand-written impls + tuple macros, the schedule keeps a single-threaded executor only.

Ordered gaps to close:

1. ~~**ECS core**~~ **(landed 2026-08)** — system params (`Res`/`ResMut`/`Query<Q>`/`Local`/`Commands`, `Option<Res<T>>`, tuples up to 8; exclusive `FnMut(&mut World)` systems still supported), `Schedule` with `ScheduleLabel` stages + `before`/`after` ordering (stable topological sort, single-threaded), and `Commands` (world-global queue drained by `World::apply_commands` after **every** system run, so a system's commands are visible to later systems in the same run; the world's change tick advances once per schedule run). `App` drives four schedules — `Startup`/`Update`/`Render`/`Shutdown` — replacing the old flat system `Vec`s; the old update-system `-> bool` exit convention is replaced by inserting the `AppExit` resource (e.g. via `Commands::insert_resource`). The low-level archetype query trait is `WorldQuery` (Bevy's name), distinct from the `Query<Q>` system param.
2. ~~**Component hooks**~~ **(landed 2026-08)** — `on_add`/`on_insert`/`on_discard`/`on_remove` per component type, registered imperatively via `World::register_component_hooks::<T>()` (no derive; `Component` stays a blanket impl). Discard hooks fire *before* the structural change (old value still readable), add/insert/remove *after*, so hooks always see a structurally consistent world and get full `&mut World` access — that's the seam relationships will use. A hook is taken out of the registry while running (no same-hook recursion; nested chains across different components fire normally). Known gaps: `spawn_batch`/`World::clear` don't fire hooks. (An `on_despawn` hook — every component still in place, fired before discards — was added with milestone 3 for linked-spawn relationship targets.)
3. ~~**Hierarchy**~~ **(landed 2026-08)** — generic `Relationship`/`RelationshipTarget` traits in `moonfield-ecs` kept in sync by hooks, registered per type via `World::register_relationship::<R>()`; `ChildOf`/`Children` (Bevy 0.20 naming) on top via `World::register_hierarchy()`. Semantics: inserting `ChildOf` links the child into the parent's auto-created `Children`; remove/replace unlinks (emptied `Children` is dropped); despawning a parent despawns children recursively (`Children::LINKED_SPAWN`); a `ChildOf` insert that would close a cycle **panics** (ancestor-chain walk — Bevy doesn't check). `Transform`/`GlobalTransform` live in `moonfield-math` (plain math types, no ECS knowledge — `moonfield-ecs` already depends on it, keeping directions acyclic); `ensure_global_transforms` + `propagate_transforms` run as normal param systems in `Update`, wired by `moonfield_app::HierarchyPlugin`.
4. ~~**Time**~~ **(landed 2026-08)** — `Time<Real>` / `Time<Virtual>` / generic `Time` resources in the new `moonfield-time` crate (bevy_time port: delta/elapsed as `Duration` + f32/f64 secs, wrapped elapsed, pause + relative speed + `max_delta` clamp on the virtual clock). `TimePlugin` inserts the resources; the winit backend advances them via `moonfield_time::update_time` once per frame before `App::update` (lazily inserting missing clocks, so the editor path works without the plugin). `Time<Fixed>` is deferred with the fixed-update schedule (known debt); `Timer`/`Stopwatch` not ported.
5. ~~**Render seam**~~ **(landed 2026-08)** — `RenderPlugin` creates the Vulkan instance + logical device at build time and inserts them as the shared `RenderDevice` world resource (`Arc`-cloneable; no driver → logs an error and inserts nothing, never panics; the instance is created with platform surface extensions, falling back to headless if unavailable). `WindowRenderer::new` now takes `&RenderDevice` and owns only window-bound objects (surface/swapchain/framebuffers/sync); presentation support of the shared device is validated per window. The editor's `EditorState` keeps only editor-only state (egui, dock layout, `WindowRenderer`) and consumes the shared device. Render-phase systems query the `World` directly — no extract/snapshot layer, no ECS dependency added to `moonfield-renderer`.
6. ~~**Scene-panel slice**~~ **(landed 2026-08)** — the editor viewport renders the ECS scene: `Camera` (fov/near/clear color; reverse-Z projection, aspect from the panel size) + `PrimaryCamera` marker + `MeshRenderer` (colored unit cube; real meshes wait for milestone 8) live in `moonfield-render::scene`, `Name` in `moonfield-ecs` (bevy_ecs parity). The viewport's scene pass queries the world directly and pushes per-cube MVP+color via push constants (`GraphicsPipeline`/`CommandBuffer` learned push constants; Y-up NDC → Vulkan via negative-height viewport). The Hierarchy panel flattens the entity tree (`collect_hierarchy`, unit-tested) and selects; the Inspector edits the selected entity's `Transform` (translation/rotation-euler°/scale drag widgets), propagating via `HierarchyPlugin`. The editor example spawns a demo scene (camera + parent/child cubes) and passes the `MOONFIELD_EDITOR_AUTO_CLOSE=5` smoke test. Deferred: depth buffer for the offscreen target (no occlusion yet), editor camera controls.
7. ~~**Mini Reflect**~~ **(landed 2026-08)** — `Reflect` trait in `moonfield-reflect` (named-field enumeration via `FieldInfo`, dynamic `field`/`field_mut` as `&dyn Reflect`, `Any` downcasts for leaves; leaf impls for f32/f64/bool/u32/i32/usize/String/glam Vec2-4/Quat/[f32;3-4]) + `#[derive(Reflect)]` in `moonfield-reflect-derive` (syn/quote; named-field non-generic structs only, `#[reflect(ignore)]`). `Transform`, `Camera`, `MeshRenderer` derive it. The inspector is fully generic: `EditorPlugin` inserts an `InspectorRegistry` world resource (`register::<T: Component + Reflect>()`), and each registered component renders as a collapsing header via the `reflect_ui` walker (nested structs recurse; leaf widgets by downcast — Quat edits as Euler-XYZ degrees). No `DynamicStruct`/type-registry/serialization.
8. ~~**Assets (sync first)**~~ **(landed 2026-08)** — `moonfield-asset` (zero-dep): `Assets<T>` slot-map store (one world resource per asset type; add/get/get_mut/remove/iter; freed slots reused with bumped generation so stale handles resolve to `None`) + `Handle<T>` (index+generation, `Copy` regardless of `T`, component/resource via the blanket impls). `SplatCloud` is the first real asset: plain data in `moonfield-renderer::splat::cloud` (wraps `GaussianScene` + source path + precomputed AABB; the crate stays ECS-free — `SplatCloudHandle(Handle<SplatCloud>)` is a plain newtype that is a component through the blanket impl, NOT via an ECS import). Loading is synchronous by the caller: the editor's Hierarchy panel has a PLY path field + Load button (`scene_io::load_splat_cloud`, unit-tested with a synthetic PLY); the entity appears in the hierarchy named after the file. Viewport rendering of splats is a **placeholder**: entities with `SplatCloudHandle` draw the cloud's AABB as a fixed-color box through the existing cube pipeline — real splat rasterization is blocked on `splat::rasterize` (itself a stub). Training/optimizer state stays outside the `World`. Deferred: async `AssetServer`/task pool, strong/weak handle refcounting, real viewport splat rendering.

Known debts (recorded, not scheduled): render-world/extract split, async `AssetServer` + task pool, real splat rasterization in the editor viewport (AABB placeholder today; `splat::rasterize` is a stub), multi-threaded executor, further proc-macro derives beyond the sanctioned `moonfield-reflect-derive`, full observers, `With`/`Without` query filters, full Reflect (`DynamicStruct`), a fixed-update schedule, `Events<T>` (to replace the hand-rolled `WindowEvents`/`InputState` frame-clearing pattern; candidate, unscheduled), audio/physics/serialization/networking.

## Coding Style & Naming Conventions

- Follow standard `rustfmt` formatting — run `cargo fmt` before committing.
- Run `cargo clippy` and resolve all warnings before opening a PR.
- Use `snake_case` for modules, functions, and variables; `PascalCase` for types and enums.
- Module files mirror their logical grouping (e.g. `device.rs`, `swapchain.rs`, `pipeline.rs` in `moonfield-render`).

## Testing Guidelines

- Tests are written alongside source using Rust's built-in `#[cfg(test)]` module convention.
- Run the full suite with `cargo test`.
- When adding a feature, add a corresponding test module in the same file or a `tests/` directory within the crate.
- Use descriptive test function names prefixed with `test_` (e.g. `test_window_control_defaults`).

## Commit & Pull Request Guidelines

Commit messages follow the **Conventional Commits** format observed in the history:

```
feat: add headless triangle recording
fix(render): box descriptor layout to prevent dangling pointer
```

- Use `feat:`, `fix:`, `chore:`, `refactor:`, etc., with an optional scope in parentheses.
- Keep the subject line under 72 characters and use the imperative mood.
- For pull requests, include a concise summary of changes, reference any linked issues, and verify that `cargo fmt`, `cargo clippy`, and `cargo test` all pass.

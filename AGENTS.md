# Repository Guidelines

Welcome to **moonfield** — a Rust workspace implementing a Vulkan RHI, a Bevy-style ECS/app framework, and an editor. This guide helps you get oriented quickly.

## Project Structure & Module Organization

The workspace is a Cargo-managed monorepo under `crates/`:

```
crates/
  moonfield/          # Binary crate — the main executable entry point (src/main.rs)
  moonfield-app/      # Bevy-style App/Plugin framework (Plugin, PluginGroup, App, Resources)
  moonfield-base/     # Shared base types and utilities
  moonfield-editor/   # Editor plugin (library crate, egui + egui_dock + egui-ash-renderer): dock panels, offscreen viewport
  moonfield-math/     # glam re-export + domain types (Dir3/Ray3d), engine clip-space conventions
                      # — the workspace math single entry (bevy_math pattern)
  moonfield-render/   # Lunar Mare — Vulkan-only rendering RHI (ash, src/vulkan/); public resource descriptions
                      # (Format, BufferUsage, VertexBufferLayout) in src/types.rs; swapchain, shaders,
                      # offscreen targets (offscreen.rs), windowed frame loop (window_target.rs)
  moonfield-renderer/ # Lunaris — scene rendering & algorithms (splat/rt/gi), RenderAlgorithm phase abstraction,
                      # 3DGS training (splat::train); targets the Vulkan RHI and re-exports it as `rhi`
  moonfield-window/   # Abstract windowing types (Window/PrimaryWindow/RawHandleWrapper components,
                      # KeyCode/MouseButton/Modifiers mirror enums, InputState/InputEvent,
                      # WindowEvents/WindowControl resources), no backend deps
  moonfield-winit/    # Windowing backend (winit), bridges winit Window → moonfield-window
                      # components (WinitWindows Entity↔WindowId map, CachedWindow field-diff sync)
```

The editor is a library crate that provides [`EditorPlugin`], a regular Bevy-style plugin composing the engine crates. `EditorPlugin` does **not** own the event loop or the window — it layers on top of `WinitPlugin` (which must be added first), reading the `WinitWindow`/`InputState`/`WindowControl`/`RawWindowEvents` resources the backend registers, and lazily building the windowed Vulkan renderer (`WindowRenderer`) + egui state on the first render tick. The editor registers a render-phase system via `App::add_render_system`; the winit backend calls `App::render` every frame after `App::update`, which drives the editor to build the egui UI and record it (plus the viewport scene) into the same swapchain. This mirrors how `bevy_egui` layers on `bevy_winit` rather than replacing it. The scene renders into an `OffscreenTarget` (final layout `SHADER_READ_ONLY_OPTIMAL`) that egui samples as a user texture in the Viewport dock panel. The egui stack is anchored to egui-ash-renderer's compatibility table (currently egui 0.33 / egui-winit 0.33 / egui-ash-renderer 0.11 + gpu-allocator / egui_dock 0.18, ash 0.38, winit 0.30) — bump them together. ECS-driven scenes are not wired into the editor yet. Setting `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` signals exit via the shared `WindowControl` after N rendered frames, which allows automated smoke tests of startup/shutdown on a machine with a display. To use the editor, add `WinitPlugin` (with `WinitSettings::continuous()` for continuous redraw) first, then `EditorPlugin`, then `app.run()`. The game (`WinitPlugin`) and editor paths share the same `WinitPlugin` runner and the `App::run()` -> `Runner` architecture.

Windows are ECS entities: the backend spawns the primary window entity in `resumed` (adopting a pre-spawned `Window` entity if user code created one at startup, Bevy-style) and attaches `Window` + `PrimaryWindow` + `RawHandleWrapper` + `CachedWindow` components. winit→ECS direction (resize/DPI/focus) writes back into the `Window` component immediately in `window_event`; ECS→winit direction (title/cursor_mode mutations by gameplay/editor code) is applied once per frame after `App::update` by `sync_windows`, which diffs the live component against the `CachedWindow` cache (Bevy's `changed_windows` pattern without change detection). `WinitWindows` (resource) holds the `Entity ↔ WindowId` mapping. There is no `WindowRequests` channel — mutate the component.

Input flows: `moonfield-winit` translates winit events into the `InputState` world resource (frame-latched; cleared each frame after the update) for ECS systems to read during the update. Keys/buttons are strongly typed (`KeyCode`/`MouseButton` mirror enums in `moonfield-window`, converted 1:1 in `moonfield-winit::converters`); auto-repeat presses arrive flagged `repeat` and never re-arm `just_pressed`; modifier state is available both as ordinary key presses and via the `Modifiers` bitflags convenience; scroll deltas keep their original `MouseScrollUnit` (no px→line folding; convert with `MOUSE_SCROLL_PIXELS_PER_LINE` = 100). `just_pressed` is frame-scoped in `on_update` and fixed-step-scoped in `on_fixed_update` (delivered to exactly one step, never lost across frames).

Window lifecycle events (`close_requested`/`resized`/`focus_*`/`scale_factor_changed`) travel on a separate channel — the `WindowEvents` world resource; every entry carries the window `Entity` (multi-window-shaped, single-window today). Exit policy mirrors Godot's `auto_accept_quit`: `CloseRequested` exits immediately by default; a caller sets `WindowControl::set_auto_exit_on_close(false)` to take over and later `WindowControl::request_exit()` (signals via the shared `WindowControl`).

The frame loop is **redraw-driven** (bevy_winit's model): `about_to_wait` only decides `ControlFlow` and requests redraws; the frame (`App::update` → `sync_windows` → `App::render` → frame-state clearing → exit check) runs inside `WindowEvent::RedrawRequested`. Update pacing is governed by the `WinitSettings` resource (`focused_mode`/`unfocused_mode`: `UpdateMode::Continuous` or `Reactive { wait, react_to_* }`; presets `game()` default / `desktop_app()` / `continuous()`), re-read every frame decision so systems can change it at runtime. External threads and UI toolkits wake an idle Reactive loop via the `EventLoopProxyWrapper` resource (`wake_up()`, sends `WinitUserEvent::WakeUp`).

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

`WindowRenderer`/`Swapchain`, `RenderPlugin`, and `moonfield-editor` are all part of the Vulkan desktop path.

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

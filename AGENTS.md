# Repository Guidelines

Welcome to **moonfield** — a Rust workspace implementing a Vulkan RHI and a TypeScript scripting runtime. This guide helps you get oriented quickly.

## Project Structure & Module Organization

The workspace is a Cargo-managed monorepo under `crates/`:

```
crates/
  moonfield/          # Binary crate — the main executable entry point (src/main.rs)
  moonfield-app/      # Bevy-style App/Plugin framework (Plugin, PluginGroup, App, Resources)
  moonfield-base/     # Shared base types and utilities
  moonfield-editor/   # Editor shell spike (egui + egui_dock + egui-ash-renderer): dock panels, offscreen viewport
  moonfield-render/   # Lunar Mare — Vulkan RHI (ash-based): device, swapchain, pipeline, shaders, headless recording,
                      # offscreen targets (offscreen.rs), windowed frame loop (window_target.rs)
  moonfield-script/   # Scripting runtime with v8 and QuickJS backends, ES modules, hot reload (src/script/), input polling API (src/input.rs)
  moonfield-window/   # Abstract windowing types (Window resource, RawHandleWrapper, InputState/InputEvent, WindowEvents/WindowControl), no backend deps
  moonfield-winit/    # Windowing backend (winit), bridges winit Window → moonfield-window resources
scripts/              # TypeScript helper scripts (e.g. record_frame.ts); moonfield.d.ts is auto-generated
```

The editor is a separate binary (`cargo run -p moonfield-editor`) composing the engine crates: `EditorPlugin` replaces the app runner with an egui-based winit event loop and owns the windowed Vulkan renderer (`WindowRenderer`). The scene renders into an `OffscreenTarget` (final layout `SHADER_READ_ONLY_OPTIMAL`) that egui samples as a user texture in the Viewport dock panel. The egui stack is anchored to egui-ash-renderer's compatibility table (currently egui 0.33 / egui-winit 0.33 / egui-ash-renderer 0.11 + gpu-allocator / egui_dock 0.18, ash 0.38, winit 0.30) — bump them together. Scripting (play mode) and ECS-driven scenes are not wired into the editor yet. Setting `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` exits the event loop after N frames, which allows scripted smoke tests of startup/shutdown on a machine with a display.

Host API bindings that touch engine layers (`record_frame`, …) live in the binary crate (`crates/moonfield/src/script_api.rs`) as the composition root — `moonfield-script` itself has no engine-layer dependencies. Bindings that only read script-owned state (the `input_*` polling API) are built into `moonfield-script` (`input::register_input_api`) and shared via an `Arc<Mutex<ScriptInputState>>` handle (`ScriptPlugin::with_input_state`). `scripts/moonfield.d.ts` is generated from the registered `ScriptApi` and kept in sync by a unit test.

Input flows: `moonfield-winit` translates winit events into the `InputState` world resource (frame-latched; cleared each frame after the update) → the script plugin mirrors it into the shared `ScriptInputState`, replays events to the `on_input` hook, and drives `on_fixed_update`/`on_update`. `just_pressed` is frame-scoped in `on_update` and fixed-step-scoped in `on_fixed_update` (delivered to exactly one step, never lost across frames).

Window lifecycle events (`close_requested`/`resized`/`focus_*`) travel on a separate channel — the `WindowEvents` world resource → the `on_window_event` hook. Exit policy mirrors Godot's `auto_accept_quit`: `CloseRequested` exits immediately by default; scripts call `app_set_auto_exit_on_close(false)` to take over and later `app_exit()` (signals via the shared `WindowControl`).

## Build, Test, and Development Commands

| Command | Description |
|---|---|
| `cargo build` | Compile all workspace crates. |
| `cargo run` | Build and run the `moonfield` binary. |
| `cargo test` | Run all unit and integration tests across the workspace. |
| `cargo clippy` | Lint the codebase with Clippy. |
| `cargo fmt` | Format all Rust source files. |
| `cargo build --features quickjs-backend` | Build the runtime with the QuickJS backend instead of the default v8 backend. |

The runtime supports two scripting backends selected via Cargo features: `v8-backend` (default) and `quickjs-backend`.

External native dependencies:

- **Slang** — `shader-slang-sys` (via `moonfield-render`) links the Slang compiler dynamically. Set `SLANG_DIR` (a prebuilt [Slang release](https://github.com/shader-slang/slang/releases) with `include/`, `lib/`, `bin/`) or install a recent Vulkan SDK (`VULKAN_SDK` is used as a fallback). The Slang shared library must also be on the runtime library path (`PATH` on Windows, `LD_LIBRARY_PATH` on Linux, `DYLD_LIBRARY_PATH` on macOS) when running binaries/tests.
- **libclang** — required by `bindgen` (used by `shader-slang-sys` and `v8`).
- The `v8` crate downloads a prebuilt static library automatically on `x86_64-pc-windows-msvc`, `x86_64-unknown-linux-gnu`, and `aarch64-apple-darwin`.

## Continuous Integration

GitHub Actions (`.github/workflows/ci.yml`) runs on pushes to `master` and on PRs, across `ubuntu-latest`, `windows-latest`, and `macos-latest` (Apple Silicon):

- `rustfmt` — `cargo fmt --all -- --check`.
- `clippy` — `cargo clippy --workspace --all-targets -- -D warnings` for both scripting backends, on all three platforms.
- `test` — `cargo test --workspace` (v8 backend) and `cargo test -p moonfield -p moonfield-script --no-default-features --features quickjs-backend` (QuickJS backend) on all three platforms.

The `.github/actions/setup-slang` composite action downloads a pinned Slang release and exports `SLANG_DIR` plus the runtime library path. On Linux, CI installs `mesa-vulkan-drivers` (lavapipe) so GPU-dependent tests (`headless_triangle`, `record_frame` extent) run for real; on Windows/macOS they skip gracefully when no Vulkan driver is present.

## Threading Model

- The script VM (V8 isolate / QuickJS runtime) is `!Send` and lives on the **main thread**, driven from the winit event loop. Scripts exist to call host APIs, and host APIs touch thread-confined resources (winit window, Vulkan device, ECS `World`) — the same reason PuerTS keeps its JsEnv on the game thread.
- Scripts never own GPU/native objects directly. `record_frame` (headless recording) is a debug-only exception; gameplay-facing host APIs must hand off via a command queue once multi-threaded rendering lands (the logic thread produces render commands, the render thread owns all Vulkan objects).
- Runaway scripts are interrupted by an execution watchdog (`DEFAULT_EXECUTION_TIMEOUT`, 5 s per top-level call); the runtime stays usable afterwards.
- The watchdog interrupts JS execution only (V8 `terminate_execution` fires at interrupt checks, QuickJS's handler runs between bytecodes); a host function blocked in a native call — e.g. a Vulkan driver call like `record_frame` — hangs the main thread past any timeout.
- For heavy JS-side compute, use one isolate per worker thread with message passing (Worker-style). Never share a runtime across threads, even under locks — the QuickJS backend has no cross-thread support at all.

## Coding Style & Naming Conventions

- Follow standard `rustfmt` formatting — run `cargo fmt` before committing.
- Run `cargo clippy` and resolve all warnings before opening a PR.
- Use `snake_case` for modules, functions, and variables; `PascalCase` for types and enums.
- Module files mirror their logical grouping (e.g. `device.rs`, `swapchain.rs`, `pipeline.rs` in `moonfield-render`).

## Testing Guidelines

- Tests are written alongside source using Rust's built-in `#[cfg(test)]` module convention.
- Run the full suite with `cargo test`.
- When adding a feature, add a corresponding test module in the same file or a `tests/` directory within the crate.
- Use descriptive test function names prefixed with `test_` (e.g. `test_script_api_roundtrip`).

## Commit & Pull Request Guidelines

Commit messages follow the **Conventional Commits** format observed in the history:

```
feat: add headless triangle recording
fix(v8): box ScriptApi to prevent dangling external pointer
```

- Use `feat:`, `fix:`, `chore:`, `refactor:`, etc., with an optional scope in parentheses.
- Keep the subject line under 72 characters and use the imperative mood.
- For pull requests, include a concise summary of changes, reference any linked issues, and verify that `cargo fmt`, `cargo clippy`, and `cargo test` all pass.

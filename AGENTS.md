# Repository Guidelines

Welcome to **moonfield** — a Rust workspace implementing a Vulkan RHI and a TypeScript scripting runtime. This guide helps you get oriented quickly.

## Project Structure & Module Organization

The workspace is a Cargo-managed monorepo under `crates/`:

```
crates/
  moonfield/          # Binary crate — the main executable entry point (src/main.rs)
  moonfield-app/      # Bevy-style App/Plugin framework (Plugin, PluginGroup, App, Resources)
  moonfield-base/     # Shared base types and utilities
  moonfield-render/   # Lunar Mare — Vulkan RHI (ash-based): device, swapchain, pipeline, shaders, headless recording
  moonfield-script/   # Scripting runtime with v8 and QuickJS backends, ES modules, hot reload (src/script/)
  moonfield-window/   # Abstract windowing types (Window resource, RawHandleWrapper), no backend deps
  moonfield-winit/    # Windowing backend (winit), bridges winit Window → moonfield-window resources
scripts/              # TypeScript helper scripts (e.g. record_frame.ts); moonfield.d.ts is auto-generated
```

Host API bindings (`record_frame`, …) live in the binary crate (`crates/moonfield/src/script_api.rs`) as the composition root — `moonfield-script` itself has no engine-layer dependencies. `scripts/moonfield.d.ts` is generated from the registered `ScriptApi` and kept in sync by a unit test.

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

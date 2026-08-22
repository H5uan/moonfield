# moonfield — Repository Guidelines

moonfield is a Rust workspace implementing a Vulkan RHI, a plugin-based ECS/app
framework, and an editor. This file carries the standing rules agents need in
every session; [docs/architecture.md](docs/architecture.md) describes the runtime
mechanisms, [crates/AGENTS.md](crates/AGENTS.md) carries workspace coding rules,
and [.agents/notes/README.md](.agents/notes/README.md) owns decision records.

## Project structure

Cargo-managed monorepo under `crates/`:

```
moonfield-app/      # Plugin-based App/Plugin framework (Plugin, PluginGroup, App, Resources);
                    # HierarchyPlugin + TimePlugin, schedules incl. the fixed-timestep loop
moonfield-asset/    # Sync-only Assets<T> store + Handle<T> (index+generation) + AssetServer
                    # (extension dispatch, path cache); no deps, no async
moonfield-base/     # Shared base types and utilities
moonfield-ecs/      # ECS world implementation (archetype storage, system params, schedules, hooks, relationships)
moonfield-editor/   # The editor — the workspace's only binary (src/main.rs). EditorPlugin
                    # (egui + egui_dock + in-house Vulkan backend in src/egui_vk.rs): dock panels, offscreen viewport
moonfield-log/      # Logging utilities
moonfield-math/     # The workspace math single entry: glam re-export + domain types (Dir3/Ray3d, Transform)
moonfield-reflect/  # Mini reflection for the editor: named fields, dynamic read/write, nesting
moonfield-reflect-derive/ # #[derive(Reflect)] proc-macro (the one sanctioned proc-macro crate)
moonfield-render/   # Lunar Mare — Vulkan-only rendering RHI (ash); see crates/moonfield-render/AGENTS.md
moonfield-renderer/ # Lunaris — scene rendering & algorithms (splat/rt/gi), RenderAlgorithm phases;
                    # Mesh + SplatCloud assets, glTF import (incl. KHR_gaussian_splatting)
moonfield-scene/    # BSN-miniature scene save/load: ResolvedScene + SceneRegistry, glTF 2.0 JSON carrier
moonfield-time/     # Time<Real>/Time<Virtual>/Time<Fixed>/Time clocks + run_fixed_main_schedule;
                    # the backend advances them per frame; TimePlugin lives in moonfield-app
moonfield-window/   # Abstract windowing types (Window components, KeyCode/MouseButton mirrors, InputState)
moonfield-winit/    # Windowing backend (winit): bridges winit Window to moonfield-window components
```

The egui stack is anchored to egui_dock's compatibility table (egui 0.36 /
egui-winit 0.36 / egui_dock 0.21, ash 0.38, winit 0.30) — **bump them
together**. The egui→Vulkan backend is in-house
(`crates/moonfield-editor/src/egui_vk.rs`); its feature spec is egui-wgpu
0.36.

## Commands

| Command | Description |
|---|---|
| `cargo build` | Compile all workspace crates. |
| `cargo run` | Build and run the editor (the workspace's only binary, `moonfield-editor`). |
| `cargo test` | Run all unit and integration tests across the workspace. |
| `cargo clippy` | Lint the codebase with Clippy. |
| `cargo fmt` | Format all Rust source files. |
| `cargo check -p moonfield-render --test headless_triangle` | Vulkan headless smoke test. |
| `python3 scripts/verify_agents.py` | Verify Agent Notes format, classification, and bilingual pairs. |

## Continuous integration

GitHub Actions (`.github/workflows/ci.yml`) runs on pushes to `master` and on
PRs, across `ubuntu-latest`, `windows-latest`, and `macos-latest`:

- `rustfmt` — `cargo fmt --all -- --check`
- `clippy` — `cargo clippy --workspace --all-targets -- -D warnings` on all
  three platforms.
- `test` — `cargo test --workspace` on all three platforms.
- `vulkan-smoke` — headless Vulkan triangle test on Ubuntu with lavapipe.
- `agent-docs` — `python3 scripts/verify_agents.py` (Agent Notes gate).

`.github/actions/setup-slang` downloads a pinned Slang release and exports
`SLANG_DIR` plus the runtime library path.

## Agent conventions

Every non-trivial change MUST add or update an Agent Note in the same commit
(see [scope](.agents/notes/README.md)). Agent Notes are decision records in
`.agents/notes/`, written in the enforced in-file format; their classification
and bilingual pairing are machine-checked by `scripts/verify_agents.py`. The
gate only verifies existing notes — whether a change needs a note is the
contributor's judgment. Skills under `.agents/skills/` encode the recurring
workflows (pre-push checks, code review, prose standard, doc translation).

## Coding & PR guidelines

- Follow `rustfmt`, keep `cargo clippy` clean, resolve warnings before opening a PR.
- Commit messages follow Conventional Commits (e.g. `feat: add headless triangle recording`, `fix(render): box descriptor layout to prevent dangling pointer`); keep the subject under 72 characters, imperative mood.
- For PRs: concise summary, reference linked issues, verify `cargo fmt`, `cargo clippy`, `cargo test` (and targeted checks) pass.

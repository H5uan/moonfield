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
moonfield-camera/   # Scene-facing camera components, targets, and projection/view math
moonfield-ecs/      # ECS world implementation (archetype storage, system params, schedules, hooks, relationships)
moonfield-editor/   # The editor — the workspace's only binary (src/main.rs). EditorPlugin
                    # (egui + egui_dock + in-house Vulkan backend in src/egui_vk.rs): dock panels, offscreen viewport
moonfield-log/      # Logging utilities (framework layer: LogPlugin needs moonfield-app;
                    # leaf crates use tracing directly — see crates/AGENTS.md)
moonfield-math/     # The workspace math single entry: glam re-export + domain types (Dir3/Ray3d, Transform)
moonfield-ml/       # ML training runtime on the RHI (Trainer, Adam, dataset, checkpoint
                    # scaffolding); Gaussian Splatting is the first method — Slang autodiff
                    # kernels compiled to SPIR-V, no external ML framework
moonfield-reflect/  # Mini reflection for the editor: named fields, dynamic read/write, nesting
moonfield-reflect-derive/ # #[derive(Reflect)] proc-macro (the one sanctioned proc-macro crate)
moonfield-rhi/   # Lunar Mare — Vulkan-only rendering RHI (ash); see crates/moonfield-rhi/AGENTS.md
moonfield-render-core/ # Selene — the render engine layer (extraction, view targets, window frame loop, RenderPlugin)
moonfield-render-feature/ # Lunaris — high-level render features (mesh/splat/rt/gi) and Core3d phases;
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
| `cargo test -p moonfield-rhi gpu_tests::headless_triangle` | Vulkan headless smoke test. |
| `python3 scripts/verify_agents.py` | Verify Agent Notes format, classification, and bilingual pairs. |
| `python3 scripts/verify_rhi_boundary.py` | Verify the rhi public API exposes no backend (ash/vk) types. |

## Continuous integration

The toolchain is pinned by `rust-toolchain.toml` at the repo root (a dated
nightly, with `rustfmt`, `clippy`, `rust-analyzer`, and `rust-src` components) —
rustup honors it locally, and CI installs it with a plain `rustup show` step.
`.github/workflows/nightly-bump.yml` rolls the date forward weekly by opening
a PR (Dependabot does not manage toolchain files).

GitHub Actions (`.github/workflows/ci.yml`) runs on pushes to `master` and on
PRs, across `ubuntu-latest` and `windows-latest` (the supported targets are
Windows and Linux):

- `rustfmt` — `cargo fmt --all -- --check`
- `rust-analyzer` — verifies the pinned language-server component is executable.
- `clippy` — `cargo clippy --workspace --all-targets -- -D warnings` on both
  platforms.
- `test` — `cargo test --workspace` on both platforms.
- `agent-docs` — `python3 scripts/verify_agents.py` (Agent Notes gate).
- `rhi-boundary` — `python3 scripts/verify_rhi_boundary.py` (no backend types
  in the rhi public API).

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

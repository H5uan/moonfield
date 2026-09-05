# Agent Note: Log-crate layering boundary — leaf crates use tracing directly

Status: implemented

[中文](2026-09-05-log-crate-layering-boundary.zh.md)

## Problem

`moonfield-log` depended on `moonfield-app` (only for `LogPlugin`'s
`App`/`Plugin` impl), and `moonfield-rhi` — the lowest-level Vulkan crate —
depended on `moonfield-log`. The transitive closure pulled the whole framework
layer (`app` → `ecs` → `time` → `base`) into the RHI, so the RHI could not be
built, tested, or reused without the ECS framework it knows nothing about.

## Decision

Adopt the reference implementation's own structure (`bevy_log` depends on
`bevy_app`; `bevy_ecs` and other low-level crates use `tracing` directly):

- `moonfield-rhi` depends on `tracing` directly; its eleven
  `moonfield_log::{error, warn, info}!` call sites are mechanically
  re-pathed to `tracing::…`. Behavior is unchanged: the macros are `tracing`
  re-exports, and output formatting belongs to the process-global subscriber
  that `LogPlugin` installs, not to the emitting crate.
- `moonfield-log → moonfield-app` stays: `LogPlugin` is framework-layer
  equipment, and its only consumer (`moonfield-editor`'s `main.rs`) already
  depends on `moonfield-app`.
- The boundary rule is recorded in `crates/AGENTS.md`: crates that must stay
  below the framework (`moonfield-rhi`, `moonfield-math`, `moonfield-base`,
  and future leaves) use `tracing` directly and never depend on
  `moonfield-log`. The `*_once!` macros live in `moonfield-log`; a leaf crate
  that ever needs them is the signal to reconsider, not to add the dependency.

## Alternatives considered

- **Move `LogPlugin` into `moonfield-app`, leaving `moonfield-log` a zero-dep
  leaf.** Rejected: it diverges from the reference implementation's layout
  (`bevy_log` is a framework crate), moves ~130 lines plus the
  tracing-subscriber/tracing-log/tracing-error dependencies and the `trace`
  feature into `moonfield-app`, and gains nothing over cutting the one harmful
  edge.
- **Feature-gate the `moonfield-app` dependency in `moonfield-log`.**
  Rejected: Cargo feature unification means the editor build enables the
  feature for the whole graph, so `moonfield-rhi` would still transitively
  pull in `moonfield-app` in every workspace build; the decoupling would only
  exist for standalone builds.

## Consequences

- `cargo tree -p moonfield-rhi` no longer contains `moonfield-app`,
  `moonfield-ecs`, `moonfield-time`, or `moonfield-log`; the RHI's dependency
  cone is `moonfield-math` (plus `moonfield-reflect`) and external crates.
- Log output format, level filtering (`RUST_LOG=moonfield_rhi=…`), and module
  targets are byte-identical — the macros are the same `tracing` macros.
- `render-core` / `render-feature` / `winit` / `editor` keep using
  `moonfield-log` (they already depend on `moonfield-app`; the edge is
  harmless there).

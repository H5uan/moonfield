# Agent Note: remove moonfield-base and unused manifest edges

Status: implemented

[中文](2026-09-06-remove-moonfield-base.zh.md)

## Problem

`moonfield-base` was a vestige: fifteen lines holding `initialize()` /
`shutdown()`, which flipped an atomic named `LOGGING_INITIALIZED` that nothing
read and nothing guarded. `App::startup`/`App::shutdown` called it out of
habit. Three more manifest edges carried no code: `moonfield-ecs` depended on
`serde` (zero references) and on `moonfield-base` (zero references), and
`moonfield-winit` still depended on `moonfield-time` after the clock advance
moved into `moonfield-app`'s `First` schedule (see
[TimeUpdateStrategy](../architecture/2026-08-27-time-update-strategy.md)).

## Decision

Delete `crates/moonfield-base` and its two call sites in
`moonfield-app::App` — the atomic guarded nothing, so no replacement lives in
`moonfield-app`. Drop the three unused manifest edges (`ecs → serde`,
`ecs → base`, `winit → time`). Remove the crate from the root `Cargo.toml`
workspace dependencies and from the crate rosters in the root `AGENTS.md`,
`crates/AGENTS.md`, and the example list in
[the log-crate layering note](../architecture/2026-09-05-log-crate-layering-boundary.md).

## Alternatives considered

- **Keep `moonfield-base` as the home for future shared primitives.**
  Rejected: an empty crate earns its keep only when it has content; a future
  primitive can recreate it in one commit.
- **Fold `initialize`/`shutdown` into `moonfield-app`.** Rejected: the
  functions' only effect was an unobserved atomic flip; inlining that into
  `App` would preserve a no-op, not a behavior.

## Consequences

- The workspace has one fewer crate; `moonfield-ecs` drops to two real
  dependency edges (`moonfield-math`, `thiserror`, `foldhash` aside from
  external crates).
- `App::startup`/`App::shutdown` now only flag initialization and run their
  schedules.
- No behavior changes; the full workspace test suite passes unchanged.

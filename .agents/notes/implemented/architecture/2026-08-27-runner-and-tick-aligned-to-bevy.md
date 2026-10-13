# Agent Note: App runner and frame tick aligned to Bevy

Status: implemented

[中文](2026-08-27-runner-and-tick-aligned-to-bevy.zh.md)

## Problem

`moonfield-app`'s `App::run` had two loop paths — a built-in spinner
(`run_updates`) and an optional plugin runner — and the winit runner
hand-wired the frame's steps (`update_time`, `sync_windows`,
`App::render`, `input.end_frame`) in `run_frame`. The runner knew about
rendering; `App::render` was a separate method tests and runners had to
remember to call; `App::run` without a runner looped forever; and a
second, mostly dead loop lived alongside the real one. Bevy's structure
is different: `App::run` always delegates to a runner (default
`run_once`), loops are the runner's job, and rendering is part of the
main tick.

## Decision

- `App::run` always calls a runner; the default is a new `run_once` that
  runs a single `App::update` tick. `run_updates` remains as the headless
  loop, but is no longer `run`'s fallback.
- `App::update` is the full tick: `First` → fixed loop → `Update` →
  `render()` → `Last`. A new `Last` schedule hosts frame-end bookkeeping.
- The runner signature now returns `AppExit` (mirroring Bevy's
  `RunnerFn`): `set_runner(impl FnOnce(&mut App) -> AppExit)`. `AppExit`
  carries a `std::process::ExitCode` (SUCCESS/FAILURE/from_code) while
  remaining the insert-to-exit resource. `moonfield-editor`'s `main`
  returns it as the process exit code.
- The winit runner's `run_frame` is now `update_time(...)` (frame-boundary
  clock advance) followed by `app.update()`; `sync_windows` and a new
  `input_end_frame` moved into `Last` systems.

Clock advance was initially kept at the frame boundary (the runner)
because the fixed-timestep tests drove the clocks deterministically via
`Time<Virtual>::advance_by` + `App::update`. It has since moved into the
schedule with Bevy's `TimeUpdateStrategy` — see
[2026-08-27-time-update-strategy.md](2026-08-27-time-update-strategy.md).

## Alternatives considered

- **Move `update_time` into `App::update`.** Rejected: overwrites the
  deterministically advanced clocks in the fixed-timestep tests.
- **Introduce `TimeUpdateStrategy` in this commit.** Deferred to a follow-up
  (now landed — see the time-update-strategy note): it is a separate API
  change to `moonfield-time`; this commit stayed mechanical.
- **Keep the runner as `FnOnce(&mut App)`.** Rejected: the exit code
  belongs in the runner contract, as in Bevy.

## Consequences

- A runner is the only way to loop; `App::run` without one runs a single
  tick and returns.
- Any runner gets time advance, rendering, window sync, and input clearing
  automatically; time advance has since moved into a `First` system via
  `TimeUpdateStrategy`.
- `App::render()` remains public for tests and embedding; it is the tail of
  `update()`.
- `run_frame` lost its hand-wired ordering — the frame's steps are now
  declared in schedules.
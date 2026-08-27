# Agent Note: TimeUpdateStrategy moves clock advance into the schedule

Status: implemented

[中文](2026-08-27-time-update-strategy.zh.md)

## Problem

After the runner/tick alignment ([2026-08-27-runner-and-tick-aligned-to-bevy.md](2026-08-27-runner-and-tick-aligned-to-bevy.md)),
clock advance was the last frame step still hand-wired into the winit
runner: `run_frame` called `moonfield_time::update_time` before
`App::update`. A headless runner got no time at all, tests could only
drive the clocks by mutating `Time<Virtual>` before each update, and the
chosen long-term shape — Bevy's `TimeUpdateStrategy` — was still absent.

## Decision

- `moonfield-time` gains `TimeUpdateStrategy` (mirroring Bevy's): the
  `Automatic` default reads the system clock; `ManualInstant(Instant)`,
  `ManualDuration(Duration)`, and `FixedTimesteps(u32)` provide
  deterministic sources.
- New `time_update_system` reads the strategy and advances
  `Real → Virtual → generic Time` through the existing
  `update_time_with_instant`/`update_time_with_duration` paths. It is
  registered in the `First` schedule by `TimePlugin`, and `TimePlugin` now
  inserts the `TimeUpdateStrategy` resource too.
- The winit runner's `run_frame` no longer touches time; it is a bare
  `app.update()` (plus redraw/exit bookkeeping). Headless `run_updates`
  advances time automatically for the first time.
- The fixed-timestep tests now drive clocks with `ManualInstant`: the
  first tick seeds the real-clock anchor (zero delta), later ticks diff
  against it. Because the manual path feeds raw deltas through the virtual
  clock, the tests disable the 250 ms `max_delta` clamp to keep the old
  no-clamp semantics.

## Alternatives considered

- **Keep clock advance in the runner.** Rejected: headless runners stayed
  without time, and the winit runner kept a non-schedule frame step —
  exactly what the alignment commit removed elsewhere.
- **Register the system only when `TimePlugin` is added.** Adopted as
  written: `time_update_system` is a `TimePlugin` system, and the
  `update_time_with_*` free functions remain for one-off tests.

## Consequences

- Every runner — winit or headless — now gets per-tick clock advance from
  one place, the `First` schedule.
- Tests and replay/networking paths get deterministic clocks via the
  `TimeUpdateStrategy` resource instead of pre-update clock mutation.
- The virtual clock's `max_delta` clamp now applies to manual strategies
  too (previously `advance_by` bypassed it); tests needing large steps
  must raise or disable the clamp explicitly.
- `update_time`/`update_time_with_instant`/`update_time_with_duration`
  remain public; the free functions still lazily insert missing clocks.
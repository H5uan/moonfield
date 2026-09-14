# Agent Note: Per-system change-detection windows

Status: implemented

[中文](2026-09-14-per-system-change-windows.zh.md)

## Problem

Change detection's module docs promised each system its own
`(last_run, this_run)` window, but query iteration compared against the
world-global window `(last_change_tick, change_tick)` — the window of the
most recent tick advance, shared by every system, with the tick advancing
once per schedule run. Two classes of writes escape detection as a result:

- A system that runs intermittently (a
  [fixed-timestep](../feature/2026-08-20-fixed-update.md) schedule at a frame
  rate above its timestep) compares against a window that has already moved
  past the writes made while it was idle.
- Within one schedule run all systems share a tick, so a write by a system
  ordered *after* a reader lands at the reader's own `this_run` boundary and
  is never seen by the reader's next run.

`CHECK_TICK_THRESHOLD` clamping was documented but never called, and the
schedule module docs stated a per-run tick advance that contradicted the
per-system window docs.

## Decision

One system run = one tick, and every system owns its window:

- `FunctionSystem::run`, `ExclusiveSystem::run`, and `SystemState::get`
  advance the world's change clock at entry. `World::increment_change_tick`
  returns the run's tick and leaves the counter at the next run's, so writes
  made after a run — by later systems, applied commands, or world
  accessors — record strictly newer ticks than everything the run observed.
- Each `FunctionSystem` remembers its `last_run` (the tick of its latest
  completed run) and fetches its params with the window
  `(last_run, this_run)`. A first run's `last_run` is 0 and the clock starts
  at 1, so it observes every existing component as new.
- The window travels through `SystemParam::refresh_window(state, last_run,
  this_run)` — a no-op default, forwarded by tuples. The `Query` param's
  state is the window (`QueryWindow`); its iterators and per-entity access
  carry the ticks. `World::query*` and `get_component_mut` keep the world
  window.
- The clock fields are interior-mutable (`Cell`), so `SystemState::get` can
  advance them through a shared world borrow.
- Schedule set anchors observe and write nothing and do not advance the
  clock.
- [Extract schedule](2026-09-09-extract-schedule.md)'s `Extract<T>` measures
  its inner param's window on the *main* world's clock via
  `ExtractState { inner, last_run }`: changes in the main world since that
  param last fetched. The render schedule's window does not apply across
  worlds.
- `Schedule::run` opens with `World::check_change_ticks`, rate-limited by
  `CHECK_TICK_THRESHOLD` against the last pass: it clamps every archetype's
  tick rows and every stored schedule's system `last_run`
  (`System::check_change_ticks`), so comparisons stay deterministic after
  the `u32` clock wraps.

The mechanics match the reference implementation's current shape
(0.20-dev): the tick advances inside the system, the returned value is the
run's tick, `SystemState` advances per fetch, and the periodic check walks
component and system ticks alike.

## Alternatives considered

- **Per-system `last_run` on a per-schedule-run tick.** The smaller patch:
  fixes the idle-schedule miss, but a same-run later writer still records
  the shared tick and escapes the reader's next window. One tick per run
  closes both.
- **Keeping `SystemState::get` observation-only (no advance).** Consecutive
  gets with no system run between them share a window and miss each other's
  writes; render commands fetch per phase item.
- **Clamping only component ticks.** A schedule that runs rarely keeps an
  ancient `last_run`; after wraparound its window exceeds `MAX_CHANGE_AGE`
  and every comparison reports changed. Clamping both sides keeps ancient
  comparisons deterministic.

## Consequences

- `Schedule::run` advances the clock by the number of systems that ran; an
  empty schedule advances nothing.
- A system sees every write made since its previous run — including writes
  by later systems in the same run, commands applied after it, and
  world-accessor writes between runs — and never re-sees its own writes.
- The world window `(last_change_tick, change_tick)` spans the
  world-accessor writes made since the most recent tick advance.
- `EntityMut` gained `is_added`/`is_changed`: its `Deref` targets the
  component directly, so `Mut`'s accessors were unreachable from per-entity
  access.
- The clock advances once per system run (plus once per `SystemState`
  fetch) instead of once per schedule run; `CHECK_TICK_THRESHOLD` clamping
  bounds the aging that follows.

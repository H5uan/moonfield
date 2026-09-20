# Agent Note: run_schedule merges mid-run registrations

Status: implemented

[中文](2026-09-19-run-schedule-merges-mid-run-registration.zh.md)

## Problem

`World::run_schedule` takes the running schedule's entry out of the
`Schedules` resource, runs it, and inserts it back. A system that calls
`World::add_systems` (or `add_sets`) for the *same* label during the run goes
through `Schedules::entry`, which creates a fresh empty schedule, and the
reinsert at the end of the run overwrites that entry — the newly registered
systems were silently lost.

## Decision

`Schedule` gains `merge_from(&mut self, other: Schedule)`: it appends
`other`'s systems (re-expanding `in_set` membership against this schedule's
set chain, without duplicating constraints the config already carries) and
absorbs `other`'s set chain, then marks the order dirty so the next run
re-sorts. `run_schedule` checks, after the run, whether the `Schedules`
resource holds an entry for the running label — that can only mean a system
registered into it mid-run — and merges that entry into the schedule being
put back instead of overwriting it. Registered systems run from the next
`run_schedule` call on, ordered by their `before`/`after` constraints as
usual. `Schedule::add_systems` shares the new `push_config` helper with
`merge_from`, so the set-anchor expansion lives in one place.

## Alternatives considered

- **Panic when a mid-run registration targets the running label.** Rejected:
  the schedule's data model (ordered config list plus stable topological
  sort) supports merging cleanly — a mid-run registration is just a late
  registration — and Bevy, the model this schedule layer is ported from,
  applies systems added to a running schedule on the next run rather than
  failing.
- **Buffer mid-run additions on the world and apply them in `run_schedule`.**
  Rejected: it adds a parallel registration channel (`World` would need a
  pending-registrations field with its own lifecycle) when the fresh
  `Schedules` entry already holds exactly the additions, typed and
  constraint-complete.

## Consequences

- Registering into the running label is well-defined: additions merge and
  take effect from the next run; nothing is silently dropped.
- A system that registers unconditionally on every run grows its schedule
  forever (each run appends again) — callers gate repeated registration
  themselves; the regression test relies on this to prove the merge.
- A rerun of the running label from inside its own run remains a no-op (the
  entry is out), unchanged.
- New test: a system registers a constrained system into its own label
  mid-run and the addition runs, in constraint order, on the next
  `run_schedule` (schedule.rs).

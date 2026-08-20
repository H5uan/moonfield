# Agent Note: Fixed-timestep schedule (Time<Fixed> + FixedMain loop)

Status: implemented

[中文](2026-08-20-fixed-update.zh.md)

## Problem

Frame-rate-dependent logic drifts: a system in `Update` sees whatever delta
the display cadence produced. Physics, simulation, and anything cumulative
need a clock that advances in fixed increments and a schedule that runs 0, 1,
or N times per frame to catch up — the reference implementation's fixed
timestep machinery. It had been deferred because there was no fixed-update
schedule to drive it.

## Decision

`Time<Fixed>` landed in `moonfield-time` (`Fixed` context: `timestep` +
`overstep`; `from_hz`/`from_seconds`/`from_duration`, `set_timestep*`,
`overstep(_fraction)`, `accumulate_overstep`/`discard_overstep`, `expend`),
together with `run_fixed_main_schedule`: accumulate the virtual delta, then
spend whole timesteps, running the fixed schedules once per step and
mirroring the generic `Time` resource to `Time<Fixed>` during each iteration
(restoring virtual time afterwards).

`App::update` runs `First`, then the fixed loop, then `Update`. The fixed
side is the full label set for API parity — `FixedFirst`, `FixedPreUpdate`,
`FixedUpdate`, `FixedPostUpdate`, `FixedLast`, plus the `FixedMain` umbrella
(systems registered under it run inside every iteration, after the five
sub-schedules). Without `TimePlugin` there is no `Time<Fixed>` resource and
the loop is a no-op, so the editor path and headless tests need no time
setup.

A deliberate structural change rode along: `TimePlugin` moved from
`moonfield-time` to `moonfield-app` (next to `HierarchyPlugin`), flipping the
dependency to app → time. The fixed loop must be driven by `App::update`,
which needs the time types — with the plugin in the time crate the dependency
would have been cyclic. `moonfield-time` is now a pure clock crate (its only
dependency is `moonfield-ecs`, for the `World` the driver function touches).
The winit backend does no fixed-step-specific input latching; fixed systems
read the same per-frame `InputState`.

Deviations from the reference, documented in the module docs: `expend` is
public (the reference keeps it crate-private because its driver lives
in-crate); there is no `RunFixedMainLoop` system-set indirection — the driver
is a hardcoded step of `App::update` since our schedule has no system-in-
Main-position concept.

## Alternatives considered

- **The driver as an exclusive system inside a `FixedMain` schedule.**
  Rejected: our schedules live in `App`, not the world, so a system cannot
  run nested schedules; the reference can do this only because its schedules
  are world resources. Moving the schedule map into the world is a much
  larger refactor for no current gain.
- **Keep `TimePlugin` in `moonfield-time` and type-erase the loop driver
  through a resource.** Rejected: an indirect function-pointer registry for
  what is a hardwired frame-phase step adds machinery to dodge a dependency
  direction that is entirely natural (app composes, time provides clocks).
- **Run fixed steps after `Update`.** Rejected: the reference runs them
  before `Update`, so `Update` systems see the post-fixed world state; the
  ordering is observable (e.g. fixed physics then render-side interpolation)
  and copying it is free.

## Consequences

- Frame-rate-independent logic has a home: `app.add_systems(FixedUpdate, …)`
  with `Res<Time<Fixed>>` (or `Res<Time>`, which is the fixed clock during
  fixed runs).
- Pause and time scaling propagate: a paused virtual clock yields zero delta,
  hence zero fixed steps.
- `moonfield-time` is dependency-light and `moonfield-app` is the
  composition point for engine plugins — the same pattern as
  `HierarchyPlugin`.
- Interpolation between fixed steps (the usual use of `overstep_fraction`)
  is possible but nothing consumes it yet.

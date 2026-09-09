# Agent Note: Schedules as world data

Status: implemented

[中文](2026-09-09-schedules-as-world-data.zh.md)

## Problem

Schedule storage lived in `App` fields — one map per world — so no system could run a schedule: the per-view execution and extract-schedule steps of the [render pass schedule redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) had no foundation. Param state (`Local` and friends) was reachable only inside a function system, while exclusive systems and the upcoming render commands need the init/fetch pair outside one.

## Decision

- `Schedules` is a `HashMap<TypeId, Schedule>` resource. `World::add_systems` registers into it (creating the resource on demand); `World::run_schedule(label)` is the only run primitive.
- `World::run_schedule` takes only the labeled entry out of the resource for the run and puts it back after. The resource itself stays in the world, so a system inside a schedule can run *other* schedules; a same-label rerun from inside its own run finds the entry gone and is a no-op.
- `App::add_systems` / `add_render_systems` / `run_schedule` / `run_render_schedule` are wrappers over the two `World` methods. The `App`-owned maps are gone, and the fixed-timestep loop's closure runs each label through `world.run_schedule` instead of split-borrowing the map.
- `SystemState<P>` holds a `SystemParam`'s persistent state — `new` initializes it, `get` fetches the param for one run — the same pair a function system holds internally.

## Alternatives considered

- **Keep the maps on `App` and hand a nested-run hook to the future camera driver.** Lost: schedule storage would have two access paths (App field and system-visible resource) and the driver would run on a bespoke callback instead of the one primitive.
- **Remove the whole `Schedules` resource for the run.** Lost: the resource is absent from the world mid-run, so a system inside could not run another schedule; taking only the entry keeps the resource present — the shape Bevy's `try_schedule_scope` uses.
- **Error on a same-label rerun (Bevy's `TryRunScheduleError`).** Lost: every current caller treats a missing schedule as a no-op (the fixed loop runs labels that may be empty), and the only rerun-while-running caller is a bug an error type cannot help find.

## Consequences

- Systems registered into a schedule while that schedule is running are overwritten by the reinsert; registration belongs at plugin build time, which matches every current caller.
- `moonfield-ecs` grows public surface: `Schedules`, `World::add_systems`, `World::run_schedule`, `SystemState`.
- Exclusive systems (`FnMut(&mut World)`) are unchanged; a driver system composes one with a captured `SystemState`.

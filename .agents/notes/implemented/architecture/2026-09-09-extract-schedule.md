# Agent Note: Extract schedule

Status: implemented

[中文](2026-09-09-extract-schedule.zh.md)

## Problem

Extraction ran as a registration-ordered closure list on `App` (`FnMut(&World, &mut World)`): no ordering constraints, no system params, and per-entity optional lookups forced hand-written loops. The [render pass schedule redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) M1 replaces it with a schedule before M2 builds the set chain on top.

## Decision

- `App::render` parks the main world for the duration of the new `ExtractSchedule`: `mem::take` the world, `World::park_main_world`, clear the render world's entities (resources persist), run the schedule, `unpark_main_world`, take the world back, then run `RenderPrepare` / `RenderQueue` / `Render`.
- `MainWorld` (moonfield-ecs) is a raw pointer in `Send + Sync` clothing. Worlds are neither (unrestricted command closures, borrow-flag columns), the `Resource` blanket impl (`Send + Sync`) excludes them, and coherence forbids a manual impl — so the parked world travels as a pointer whose validity is the parking contract: only `Extract` dereferences it, and only while the schedule runs.
- `Extract<T>` (render-core) is the Bevy-shaped system parameter: `T::fetch` runs against the parked main world; the item holds the resource cell's shared borrow so the parked world cannot be replaced mid-run.
- Extract systems write the render world through `Commands`, applied after each system so later extract systems observe earlier spawns — the same ordering the closures had. The closure list and `App::add_extract_system` are gone; every extractor migrated (cameras, windows, `extract_with_transform`, mesh and shader assets, the editor frame).

## Alternatives considered

- **A bare `Extract` parameter exposing `&World` of the main world.** Lost: the query engine could not express the extraction shapes; shipping the [composable WorldQuery](2026-09-09-composable-world-query.md) instead keeps extraction type-directed.
- **Park the world behind `Arc<Mutex<World>>`.** Lost: satisfying the `Resource` bounds would require `Send` command closures across every call site, plus lock ceremony on a single-threaded seam.
- **Keep the closures and add the schedule with M2.** Lost: the set chain consumes `ExtractSchedule`; deferring it serializes the milestones for no saving.

## Consequences

- Extract systems compose with system params and ordering constraints like every other system; registration is `add_render_systems(ExtractSchedule, ...)`.
- The per-frame render-world entity rebuild stays (entity sync remains out of scope — the redesign's red line).
- moonfield-app stays `#![forbid(unsafe_code)]`: the pointer work lives in moonfield-ecs (`MainWorld`) and render-core (`Extract::fetch`).

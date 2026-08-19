# Agent Note: Archetype-based ECS storage

Status: implemented

[中文](2026-08-19-ecs-archetype-storage.zh.md)

## Problem

The early `World` stored components per entity (`EntityId` keyed), so a query
scanned every entity and filtered by component type at runtime. Spawn and
component insert/remove were hot-path O(entities) or required expensive
bookkeeping, component data was scattered in memory (cache-unfriendly for the
tight loops the app and renderer run), and there was no natural way to page
entities by their component set. The restructure tracked by
[#24](https://github.com/H5uan/moonfield/pull/24) existed to replace this with
archetype storage.

## Decision

`moonfield-ecs::world2::World2` stores entities in **archetypes**: an entity's
component set selects the archetype it lives in, and each archetype stores
components in flat, tightly packed slices (`archetype.rs`). Supporting pieces:

- `bundle_to_archetype` caches the `TypeId` bundle → archetype lookup;
  `insert_edges` / `remove_edges` cache the migration edge between archetypes.
- `entities` map holds `EntityId` → (archetype index, row index); zero is
  reserved for entities with no components, flushed in batch by `World2::flush`.
- `spawn_at` reuses a chosen `Entity` (the winit backend adopts a pre-created
  window entity this way).
- Change detection is **tick-based**: the world owns `change_tick` /
  `last_change_tick`, `increment_change_tick` advances the clock once per
  schedule run, and the clock starts at 1 so a system's initial `last_run = 0`
  observes every component as changed.
- `query` / `query_mut` iterate through the `Query` trait over matching
  archetypes with borrow checking (`borrow.rs`).

The old `World` still exists during the migration; the two lives are being
unified under one name once the migration completes.

## Alternatives considered

- **Keep the entity-keyed map.** Rejected: component filtering and insert/remove
  cost grew with total entity count instead of with the matching subset, and
  scattered component slices defeated the tight loops this engine targets.
- **Adopt bevy_ecs wholesale.** Rejected: the workspace builds its own learning
  line of ECS in `moonfield-ecs`; pulling a full dependency would have surrendered
  the design decisions (e.g. simple tick windows) this crate wants to own.
- **Sparse-set storage (entity-side SOA) instead of archetype.** Rejected: it
  wins on single-component insert/remove but loses on dense archetype iteration,
  which is the dominant pattern here.

## Consequences

- Queries iterate only matching archetypes and touch contiguous memory; spawn /
  despawn are batch-allocated through `flush`, which is the single point where
  entity ids get their row indices — reason about it precisely.
- `World2` is a temporary migration name; unifying with the old `World` under
  one name is a follow-up.
- The tick window `(last_change_tick, change_tick)` spans exactly the writes
  since the previous schedule run — do not advance the clock mid-run.
# Agent Note: Composable WorldQuery

Status: implemented

[中文](2026-09-09-composable-world-query.zh.md)

## Problem

The query engine implemented each shape as a hand-written iterator — `&T`, `&mut T`, the three two-component pairings, `Option<&T>` alone — with no tuple beyond arity two and no `Option` inside a tuple. Extract systems need `(&Camera, &GlobalTransform, Option<&CameraTarget>, Option<&PrimaryCamera>)`; the missing shapes are why `extract_cameras` had to hand-loop with per-entity `get_component` calls.

## Decision

- `WorldQuery` is the Bevy-style composition contract: each element decides archetype membership (`matches`), borrows its columns for the iterator's lifetime (`borrow_fetch` / `release`, the archetype borrow flags), and produces one item per row (`fetch`). The `READ_ONLY` const says whether the query contains `&mut T`.
- One generic `QueryIter` replaces the per-shape iterators. `World::query` / `query_mut` / `query_filtered(_mut)` and the `Query` system param are thin entries over it; the shared entries reject mutable queries (the exclusive entries are `query_mut` and `iter_mut`), matching the previous panics.
- Tuples compose all three steps conjunctively (a macro, arity 0–8, matching the system-param tuples); `Option<Q>` matches every archetype and yields `None` rows where `Q`'s column is absent.
- Per-entity `Query::get` stays single-component only.

## Alternatives considered

- **Grow the per-shape iterator set.** Lost: every new combination is another hand-written iterator; the combinatorics are exactly what the composition contract removes.
- **Bevy's `State` / `Fetch` / `set_archetype` machinery over an unsafe-cell world.** Lost: moonfield's safety model is runtime borrow flags on archetype columns; the three-step protocol delivers the same composition under that model without adopting the unsafe-cell substrate.
- **Suppress the need with an `Extract` parameter exposing `&World`.** Lost: it works around the missing shapes rather than shipping them, and gives up type-directed query declaration for every future extractor.

## Consequences

- Query call sites are unchanged (`world.query::<Q>()` reads the same); `Q::Iter` disappears from the public surface — `QueryIter<'w, Q>` is the iterator.
- Every pre-existing query test passes on the new engine; five new tests pin tuple conjunction, `Option` in shared and mutable tuples, standalone `Option` parity, and the shared-entry rejection of mutable access.
- The [extract schedule](2026-09-09-extract-schedule.md) is the first consumer of the new shapes.

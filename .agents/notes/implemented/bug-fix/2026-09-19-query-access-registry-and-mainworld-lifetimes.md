# Agent Note: Query-param conflict detection and MainWorld lifetimes

Status: implemented

[中文](2026-09-19-query-access-registry-and-mainworld-lifetimes.zh.md)

## Problem

Two soundness holes in `moonfield-ecs`:

1. `QueryIter` releases the archetype columns' borrow flags when it drops,
   but the items it yields borrow the world for `'w` and outlive the
   iterator. `let refs: Vec<&A> = q1.iter().collect();` keeps `&A` alive
   after the flags are released, and a sibling param `q2: Query<&mut A>` in
   the same system can then call `iter_mut()` and alias them — no check stood
   between the params of one system. (The same gap for `fetch_mut_cell` was
   closed by marking it `unsafe` in
   [spawn_at hooks and mutable-query safety](2026-09-05-spawn-at-hooks-and-mutable-query-safety.md);
   the `Query::iter` entry point stayed reachable from safe code.)
2. `MainWorld` — the main world parked in the render world during extraction
   — exposed `unsafe fn world<'w>(&self) -> &'w World` with a caller-chosen
   lifetime, so one parked resource could mint any number of overlapping
   `&World` references detached from the resource borrow.

## Decision

- The world carries an `AccessRegistry` (per-component read counts plus a
  write set, keyed by `TypeId`). `WorldQuery` gains
  `register_access`/`unregister_access`; `Query::fetch` registers the query's
  component access and `Query::drop` unregisters it, so the registry always
  mirrors exactly the live params. Registering read access over a live write,
  or write access over any live access, panics at fetch time — before any
  iteration and before items can outlive the flags. Fetch stages registrations
  in a scratch clone of the registry, so a query that conflicts with itself
  (`Query<(&A, &mut A)>`) panics without leaving a partial registration behind.
  The check is component-level and filter-blind: `Query<&A, With<X>>` conflicts
  with `Query<&mut A, Without<X>>` even where the filters are disjoint.
- `MainWorld::world(&self) -> &World` ties the returned reference to the
  resource borrow, and `world_mut(&mut self) -> &mut World` covers the
  exclusive case (no caller needs it yet), so `&MainWorld` yields only shared
  borrows with the guard's lifetime and `&mut MainWorld` yields at most one
  `&mut World`. `Extract::fetch` (moonfield-render-core) re-expresses the
  guard-bound lifetime as the fetch lifetime through a raw-pointer round trip,
  justified by holding the `Ref<MainWorld>` in the returned item. The parking
  mechanism is unchanged.
- `World::query_filtered`/`query_filtered_mut` were audited for the same
  hole: they take `&self`/`&mut World`, so the borrow checker already gates
  every access they hand out; no change.

## Alternatives considered

- **Lending iterator: bind each item to the `&mut self` borrow of
  `QueryIter::next`.** Rejected for the reason recorded in
  [spawn_at hooks and mutable-query safety](2026-09-05-spawn-at-hooks-and-mutable-query-safety.md):
  Rust's `Iterator` cannot yield items borrowing `&mut self`, so this means a
  lending-iterator redesign of the whole query engine — and it would break
  the `iter().collect()` pattern call sites rely on. The registry closes the
  same hole while keeping the iterator API.
- **Run-scoped registration: clear the registry before each system run
  instead of unregistering on drop.** Rejected: params are also fetched
  outside `FunctionSystem::run` (direct `SystemParam::fetch` in tests,
  `SystemState::get` in render commands), and a run-scoped clear never
  reaches those paths, leaving stale registrations that falsely conflict with
  later fetches. Drop-scoped registration covers every fetch path uniformly
  with no clearing point to miss.
- **Filter-aware conflict detection (Bevy's `With`/`Without`
  disjointness).** Rejected: it needs each filter's component set in the
  check; no current system relies on filter-disjoint param pairs, and the
  conservative panic fails loud instead of silently allowing a real conflict.

## Consequences

- A system whose `Query` params overlap incompatibly panics at fetch with a
  `conflicting Query params` message, before any iteration; the
  `iter().collect()` pattern keeps working for non-conflicting params.
- From `&MainWorld` only shared `&World` references with the borrow's
  lifetime are derivable; mutable access requires `&mut MainWorld`.
- The registry is per-`World`: `Extract<Query<…>>` registers into the parked
  main world, and the params' drop releases the registration, so sequential
  extract systems never conflict.
- New tests: read/write and write/write param conflicts panic at fetch, an
  intra-query conflict rolls its registration back, registrations release
  when params drop (system.rs), and the park/world/unpark roundtrip
  (world.rs).

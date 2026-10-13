# Agent Note: Per-system query archetype cache and wired edge tables

Status: implemented

[中文](2026-09-19-query-archetype-cache-and-edge-tables.zh.md)

## Problem

Two recurring costs in `moonfield-ecs` grew with the world's history rather
than with its live data:

1. Every `Query::iter()`/`iter_mut()` re-scanned the whole archetype list and
   allocated a fresh `Vec` of matches. The archetype list grows monotonically
   and empty archetypes are never collected, so the scan cost grew over the
   app's lifetime even at steady state.
2. `World` carried the ported `insert_edges`/`remove_edges` tables (and an
   `InsertTarget` value type) without ever reading them: every
   `insert_component`/`remove_component` rebuilt the target archetype's type
   set from scratch (a `Vec<ComponentMeta>` plus a `Box<[TypeId]>`, a sort,
   and a whole-slice hash lookup), `insert_bundle` added an O(n×m) `contains`
   merge, and `spawn`/`spawn_at`/`despawn` allocated a `Vec<TypeId>` per call
   that only fed the lifecycle-hook loops. A crate-wide
   `#![allow(dead_code)]` hid all of this plus more (`SpawnColumnBatchIter`
   and its `Entities::alloc_many` support, `Archetype::merge`/`move_to`, and
   an `AtomicU64` world id that was never read).

## Decision

- `Query<Q, F>`'s `SystemParam::State` is a new `QueryState`
  (system.rs): the matching archetype indices, the world id, and the
  archetype count the cache was built against, alongside the per-system
  change-detection window the state already carried (the window is what
  `refresh_window` rewrites before each fetch; the cache checks below run in
  `fetch` itself). `fetch` rebuilds the match list when the world id differs
  or the count changed — the archetype list is append-only, so a count match
  means the cache is complete. Iteration builds its fetch list from the
  cached indices (`QueryIter::new_cached` / `new_shared_cached` in query.rs)
  with one exact-capacity allocation and no scan. `Query` gains a second
  lifetime parameter (`Query<'w, 's, Q, F>`) borrowing the state, like
  `MessageReader` borrows its `MessageCursor`.
- The unused `World.id` field became the cache's world-mismatch guard: a
  process-wide counter hands each `World` a unique id (starting at 1; 0 marks
  an unbuilt `QueryState`). This covers a `SystemState` being fetched against
  different worlds (e.g. tests, or `Extract` reading the parked main world),
  which a count check alone cannot distinguish.
- The imperative `World::query`/`query_mut`/`query_filtered`/`query_filtered_mut`
  entries keep scanning: they are one-shot calls with no persistent state to
  key a cache against, and the scan is their documented cost.
- The edge tables are wired as `(archetype id, TypeId) → target archetype`
  memoizations of the type-set union/difference. `insert_component` and
  `remove_component` consult `insert_edges`/`remove_edges`; `insert_bundle`
  consults a separate `bundle_edges` keyed by the bundle's static type id.
  Bundles without a static key (`DynamicBundle::key() == None`) always
  recompute. Edges never invalidate because archetypes are append-only.
- `insert_bundle`'s union of the old metas with the bundle metas is a merge
  of two sorted lists (both are sorted by `ComponentMeta::cmp`) instead of
  the quadratic `contains` loop.
- `spawn_inner`/`spawn_at`/`despawn` collect the component id `Vec` only when
  the world's hook registry is non-empty, so hook-free worlds skip the
  allocation entirely (`Vec::new` does not allocate).
- The dead code the blanket allow was hiding is deleted
  (`SpawnColumnBatchIter`, `Entities::alloc_many`/`finish_alloc_many`/
  `resolve_unknown_gen`/`AllocManyState`, `Archetype::merge`/`move_to`, the
  `InsertTarget` type) or, where it is deliberate in-progress scaffolding
  (entity-ref/component-ref access, column-batch spawning, dynamic clone
  bundles, allocator introspection for future serialization), carries a
  targeted `#[allow(dead_code)]` with a comment at the item. The crate-wide
  `#![allow(dead_code)]` is gone, so new dead code warns.

## Alternatives considered

- **Cache the fetches, not just the indices (Bevy's `QueryState`
  shape).** Rejected for now: a fetch holds the archetype's column borrow
  flags, so it cannot live in `'static` param state; borrows must be retaken
  per iteration anyway. Caching column indices per archetype would save the
  per-archetype column lookups, but those are binary searches over a handful
  of columns — the scan was the cost that grew unboundedly.
- **A borrow-on-demand iterator that stores no fetch list at all (true zero
  allocation).** Rejected: it releases each archetype's borrow flags when the
  iterator advances past it instead of holding them until drop, which widens
  the window in which a yielded item outlives its column's borrow flag — the
  exact gap the fetch-time
  [access registry](../bug-fix/2026-09-19-query-access-registry-and-mainworld-lifetimes.md)
  exists to close from the param side. One exact-capacity `Vec` per iteration
  is the floor that keeps the current borrow semantics.
- **Delete the edge tables instead of wiring them.** Rejected: wiring is
  what the ported fields existed for, and the win is structural, not just
  allocation count — a `(u32, TypeId)` lookup with the pass-through hasher
  replaces building and hashing a whole type-set key per insert/remove.
  Hierarchy maintenance (`ChildOf` inserts/removes) hits the same few edges
  repeatedly.
- **Key bundle inserts in `insert_edges` too.** Rejected: the blanket
  `Component` impl makes a tuple type usable as a single component
  (`insert_component::<(A, B)>`), so a bundle's type id and a component's
  type id can be identical — the key spaces must stay in separate maps.

## Consequences

- Steady-state system queries no longer scale with the world's total
  archetype count; the per-iteration cost is proportional to the matched
  set, with one exact-capacity allocation per iterator.
- Structural mutations (`insert_component`/`insert_bundle`/`remove_component`)
  hit a hash lookup on repeat shapes and pay the type-set construction once
  per (archetype, component/bundle) pair.
- `Query` is fetched against the world whose id is in its `QueryState`;
  mixing one state's fetches across worlds rebuilds the cache rather than
  reading the wrong archetypes.
- `Query`'s extra lifetime parameter stays inside the `SystemParam`
  machinery (the `QueryWindow` state became `QueryState`); downstream crates
  (moonfield-app, moonfield-render-core, moonfield-editor) compile unchanged.
- New tests: a system query picks up archetypes created between runs, a
  filtered query matches an archetype that appears after its first run, and
  one `SystemState` shared across two worlds rebuilds on the world-id check
  (system.rs).

# Agent Note: spawn_at hook coverage, hook panic safety, and mutable-query safety contract

Status: implemented

[中文](2026-09-05-spawn-at-hooks-and-mutable-query-safety.zh.md)

## Problem

Three correctness gaps in `moonfield-ecs`'s lifecycle and query machinery,
surfaced by an architecture review:

1. `World::spawn_at` overwriting a **live** entity dropped its old components
   without firing any lifecycle hooks. `ChildOf`'s unlink hook never ran, so a
   parent's `Children` could be left pointing at an entity that no longer
   carries `ChildOf` — a silently broken hierarchy invariant.
2. `WorldQuery::fetch_mut_cell`/`fetch_mut_cell_with` were safe (though
   `#[doc(hidden)]`) trait methods producing `Mut<'w, T>` items from a shared
   `&World`. Column borrow flags are released when the *iterator* drops, while
   yielded items stay valid for `'w`: collecting the items, dropping the
   iterator, and fetching the same column again aliases `&mut` in safe code.
3. A panicking hook was permanently lost: `fire_hook` takes the hook out of
   the registry (the recursion guard) and only restored it on the success
   path, so one panic unregistered the hook — and with it the hierarchy
   invariants it maintains.

## Decision

- `spawn_at` now fires the despawn sequence (`on_despawn` → `on_discard`) for
  the old components **before** calling `alloc_at`, because `alloc_at`
  invalidates the entity's location until `spawn_inner` re-places the row and
  hooks running with an invalidated location cannot even read the entity. If a
  hook despawned the entity, the spawn aborts gracefully (the crate's standing
  convention for hooks mutating the hooked entity). The old row is then
  removed (dropping values) and `on_remove` fires per old component, mirroring
  `despawn`; the new components fire `on_add` → `on_insert` via `spawn_inner`
  as before.
- `fetch_mut_cell`/`fetch_mut_cell_with` are now `unsafe fn` whose contract
  requires that produced items not outlive the exclusive borrow gating column
  access. The two safe entry points justify the call: `fetch_mut` takes
  `&mut World` (items keep that borrow alive) and `Query::iter_mut` takes
  `&mut self` on the `Query` param (items keep the query borrow alive).
- `fire_hook` wraps the hook call in `catch_unwind`, restores the hook into
  the registry, then `resume_unwind`s, so a panicking hook is never lost and
  the panic still propagates.

## Alternatives considered

- **Tie query item lifetimes to the iterator (lending iterator).** Rejected:
  Rust's `Iterator` cannot yield items borrowing `&mut self`, so this means a
  lending-iterator redesign of the whole query engine to fix one contract.
  Marking the method `unsafe` keeps the GAT design and moves the invariant
  (previously a comment) into the type system; see
  [query-filters](../feature/2026-08-19-query-filters.md) for the trait's
  evolution.
- **Keep `spawn_at` hook-free and document the restriction.** Rejected:
  silently breaking hierarchy links is worse than paying the despawn hook
  sequence on a path that overwrites live entities; `despawn` already pays it.
- **RAII guard restoring the hook through a raw world pointer.** Rejected:
  `catch_unwind` + `resume_unwind` achieves the same panic safety without new
  `unsafe` (and `panic = abort` builds are unaffected either way).

## Consequences

- Overwriting a live entity via `spawn_at` keeps hierarchy invariants:
  overwriting a child unlinks it from its parent's `Children`; overwriting a
  parent despawns its children (linked-spawn), matching `despawn` semantics.
  Covered by new tests in `hierarchy.rs` and `hooks.rs`.
- The `Commands::spawn` path (reserve + `spawn_at` on a reserved id) is
  unchanged: a reserved id has no components, so no despawn hooks fire.
- Misusing `fetch_mut_cell` now requires `unsafe`; the in-crate callers carry
  SAFETY comments.
- A panicking hook is restored before the panic resumes, so hook-backed
  invariants survive a hook failure.

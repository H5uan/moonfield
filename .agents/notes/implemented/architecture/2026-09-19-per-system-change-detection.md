# Agent Note: Added/Changed query filters and seed-based transform propagation

Status: implemented

[中文](2026-09-19-per-system-change-detection.zh.md)

## Problem

The per-system change-detection windows (see
[the windows note](2026-09-14-per-system-change-windows.md)) gave every system
its own `(last_run, this_run)` window and made `Ref`/`Mut` items tick-aware,
but no query consumed the windows as a *filter*: `Added<T>`/`Changed<T>` did
not exist, so the stored per-component ticks were only observable imperatively.
Meanwhile `ensure_global_transforms`/`propagate_transforms` walked the entire
hierarchy in both `Update` and `PreRender` every frame, rewriting every
`GlobalTransform` unconditionally — the exact workload change detection exists
to skip.

## Decision

- `QueryFilter` is split into an archetypal part and a per-row part. The
  archetypal part (`matches_component_set`) is unchanged and remains what the
  per-system `QueryState` archetype cache memorizes. The per-row part is a GAT
  (`RowState<'w>`) built per matched archetype at iterator construction and
  evaluated per entity in `QueryIter::next`; `With`/`Without`/`Or` use `()`
  and inline away, while `Added<T>`/`Changed<T>` hold the component's tick
  column pointer plus the querying system's window and compare the row's
  added/changed tick. The cache therefore memorizes archetype matches only,
  never tick verdicts. Filters compose exactly as before: tuples conjoin,
  `Or` disjoins, `()` matches everything.
- `QueryIter` is generic over `F` (default `()`, so `World::query` and other
  existing signatures are unchanged). `Query::get` applies the row predicate
  too, on top of the remote `QueryGetGuard` path: locate the row, evaluate
  the filter's per-row state for it, then run the usual per-entity fetch.
  Tick filters register a read on their component's tick column in the
  world's access registry at fetch time.
- `propagate_transforms` is seed-driven on top of the remote composed-query +
  `Local` worklist structure (not the old multi-query shape): a `seeds` param
  `Query<&Transform, Or<(Changed<Transform>, Changed<ChildOf>)>>` scans only
  entities changed since the propagation system's previous run, while the
  composed `nodes` query answers per-entity lookups. Each seed's global is
  recomputed — a root takes its local affine, a non-root composes onto its
  parent's *stored* global — and the seed's whole subtree is rewritten
  through the worklist without further tick checks (a change in a chain moves
  every descendant). Correctness is order-independent: if a seed's ancestor
  also changed, the ancestor's cascade recomputes the seed again with the
  same result. Unchanged subtrees are never entered. Unlinking a `ChildOf`
  (the one change that leaves no fresh tick on the entity) is covered by the
  `ChildOf` discard hook marking the orphan's `Transform` changed.
  `ensure_global_transforms` scans `Query<&Transform, Added<Transform>>`
  instead of all transforms. Both systems stay registered in `Update` and
  `PreRender`, unchanged in name and ordering, so the editor's
  `editor_prepare.before(&ensure_global_transforms)` contract is untouched.

## Alternatives considered

- **Bevy 0.20's dirty-bit propagation (`TransformTreeChanged` +
  `mark_dirty_trees` + `RemovedComponents<ChildOf>`).** Rejected: it needs
  removal tracking (we have no removed-components channel) and a second
  system propagating a marker component up the ancestor chain. The seed scan
  covers the same cases through the ticks we already store — `Changed<ChildOf>`
  catches attaches and reparents, the discard hook catches orphans.
- **Reverting propagation to the pre-rebase multi-query shape (separate
  `transforms`/`childofs`/`children`/`globals` params).** Rejected: the
  remote base already resolves each node in one composed query with a
  worklist cascade; adding a `seeds` param to that structure keeps one node
  layout and one cascade loop instead of reintroducing four lookups per
  entity.
- **`Added<GlobalTransform>` in the seed filter (Bevy parity).** Rejected:
  it would register a read on `GlobalTransform` that conflicts with the
  propagation system's own `&mut GlobalTransform` element at fetch time (the
  access registry does not subsume a filter's read under a sibling element's
  write). `Added<Transform>` on `ensure_global_transforms` already covers the
  spawn path that member exists for.
- **Subsume filter reads under the query's own write access (Bevy's
  `FilteredAccess` merge), so `Query<&mut T, Changed<T>>` is legal.**
  Rejected as machinery without a caller: tick filters register plain reads,
  so a `Changed<T>` filter combined with `&mut T` in one query panics at
  fetch like any other read/write conflict. No system in the workspace wants
  that shape; the propagation queries are read-only on their filtered
  components.

## Consequences

- A quiet frame costs one seed scan per schedule instead of a full tree walk
  rewriting every global, and `GlobalTransform` changed ticks now mean "the
  pose actually moved" — the signal a future extraction-side change filter
  would consume.
- Behavior for changing trees is unchanged: the same runs recompute the same
  globals, verified by the pre-existing propagation tests and the
  render-core extraction test that drives a `PreRender` transform write.
- `Query::get` on a tick-filtered query applies the per-row predicate; the
  imperative `World::query_filtered` entries evaluate tick filters against
  the world-global window.
- Known boundary: `Query<&mut T, Changed<T>>` in one query panics at fetch as
  a read/write conflict — accepted, no current caller needs it.
- New tests: `Added`/`Changed` per-run delivery over the world window and
  `Or` composition; two systems with different windows see different change
  sets; per-row `Query::get`; propagation skips unchanged subtrees (probed by
  a `Changed<GlobalTransform>` counting system: empty frame 0 writes, a root
  move rewrites only its subtree), reaches a child attached under an
  unchanged parent, and recovers an orphaned entity.

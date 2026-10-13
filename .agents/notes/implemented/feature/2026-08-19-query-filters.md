# Agent Note: Archetypal query filters (With/Without/Or)

Status: implemented

[中文](2026-08-19-query-filters.zh.md)

## Problem

Queries could only express "entities that have exactly these components" —
the fetch itself. Common selection patterns like "everything with a Transform
that is not a child" or "anything with A or B" required either fetching
components that are never read (paying borrow and bandwidth for nothing) or
hand-rolling membership checks inside the loop body, per entity, every frame.

## Decision

`moonfield-ecs` gains `filter.rs`: a `QueryFilter` trait with `With<T>`,
`Without<T>`, `Or<(…)>` (disjunction), tuples up to eight (conjunction), and
`()` (no filter), named after the reference implementation's query filters.

- All filters are **archetypal**: `QueryFilter::matches_component_set` is
  evaluated once per archetype against its component type set, at iterator
  construction. Filtering never touches per-entity data, so a filtered query
  costs one probe per archetype — this is the natural fit for archetype
  storage, and the reason the filter surface is deliberately this small.
- The `Query` system param gains a defaulted second type parameter:
  `Query<Q, F = ()>`. `iter`/`iter_mut`/`get` all respect `F` (`get`
  rejects entities in non-matching archetypes before fetching).
- The `WorldQuery` trait's `fetch`/`fetch_mut_cell` became provided methods
  delegating to new `fetch_with`/`fetch_mut_cell_with`, which take an
  archetype predicate; every iterator constructor now computes its hit list
  through that predicate. `Option<&T>`'s iterator previously walked every
  archetype (no hit list); it now records the passing archetypes at
  construction.
- Imperative access mirrors it: `World::query_filtered::<Q, F>()` and
  `query_filtered_mut::<Q, F>()`.
- The trait's probe is a `&dyn Fn(TypeId) -> bool` closure rather than the
  archetype itself, so the private `Archetype` type never appears in the
  public signature.

## Alternatives considered

- **Per-entity filters.** Rejected: nothing in `With`/`Without`/`Or` needs
  per-entity evaluation on archetype storage; adding that axis would only
  invite accidentally-slow filters. Change-detection filters (`Added`,
  `Changed`) would need it and remain unported.
- **Type-level filter composition inside `WorldQuery`.** Rejected: making
  the query item itself carry the filter (e.g. a `(Q, F)` query pair) would
  entangle fetch logic with selection; keeping `F` on the `Query` param
  matches the reference's shape and leaves `WorldQuery` untouched in spirit.
- **Exposing `Archetype` in the filter trait.** Rejected: the module is
  crate-private; a `TypeId` probe closure keeps the boundary intact at the
  cost of one indirect call per archetype.

## Consequences

- `Query<&T>` (one parameter) keeps working unchanged — `F` defaults to
  `()`.
- Filtered and unfiltered iterators share constructors; there is one code
  path, and the pass-all case compiles to the same scans as before.
- `Added`/`Changed` filters, and filters on `Entity` fetches beyond the
  single-component `get` shapes, are not ported; they need change ticks
  consulted per entity and are easy to add later on this seam.

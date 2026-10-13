# Agent Note: doc and metadata alignment fixes

Status: implemented

[中文](2026-09-19-doc-metadata-alignment.zh.md)

## Problem

A pass over durable prose found four mismatches with the shipped code:

1. The root `AGENTS.md` roster described `moonfield-ecs` without the
   Transform/GlobalTransform propagation it carries (`hierarchy.rs`), even
   though [docs/architecture.md](../../../../docs/architecture.md) records the
   propagation systems and the deliberate `moonfield-ecs` → `moonfield-math`
   dependency direction.
2. `moonfield-math`'s `Cargo.toml` carried the workspace's only
   Chinese-language manifest comment; every other `Cargo.toml` comment is
   English (bilingual pairing applies to Agent Notes only).
3. `sphere_from_points(&[])` silently returned a NaN-centered sphere while
   the sibling `aabb_from_points` returns `None` for empty input — the
   divergence was undocumented.
4. The `Query::get` panic for tuple and `Option` query shapes named neither
   the offending shape nor a working alternative.

## Decision

Fix prose, not behavior. The `AGENTS.md` roster line now names the
propagation and the math dependency. The manifest comment is translated.
`sphere_from_points` documents that an empty slice divides the zero-sum by
the (zero) point count and yields NaN center components with a `0.0` radius.
The `Query::get` panic message includes the unsupported shape's type name
and points at `World::get_component`/`World::get_component_mut` and at
iterating the query as the working paths. The `docs/architecture.md` claim
that `spawn_batch` and `World::clear` do not fire hooks was re-verified
against `world.rs` and stands unchanged.

## Alternatives considered

- **Make `sphere_from_points` return a degenerate-but-finite sphere for
  empty input.** Rejected: inventing a sentinel hides caller bugs much the
  way NaN does, and switching to `Option<BoundingSphere>` would be a
  breaking signature change; documenting the NaN contract follows the
  [doc-claims-vs-code audit](2026-09-19-doc-claims-vs-code-audit.md) rule of
  describing shipped behavior rather than redesigning it.
- **Implement `Query::get` for tuple and `Option` shapes.** Rejected: the
  `WorldQuery::get_entity` doc ports those shapes when a caller needs them,
  and none does today, so the panic is the correct failure; only its
  message needed work.

## Consequences

- The root roster, the math manifest, and the bounding/query docs agree
  with the code; no runtime behavior changed.
- A `Query::get` panic on an unsupported shape now reports the shape's type
  name and the supported alternatives, so the failure is actionable without
  reading `query.rs`.

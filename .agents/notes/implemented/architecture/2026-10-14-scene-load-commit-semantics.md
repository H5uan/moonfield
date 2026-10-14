# Agent Note: Scene load commit semantics

Status: implemented

[中文](2026-10-14-scene-load-commit-semantics.zh.md)

## Problem

The [architecture contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)'s
second item: scene loading was not failure-atomic. `load_scene` resolved and
applied one root at a time, so an invalid later root stranded the earlier
roots' entities in the destination world. `ResolvedScene::apply` spawned
entities before its fallible template builds completed, so a missing asset or
a failed child template left a partial subtree behind.

## Decision

**Resolution is pure and comes first.** `load_scene` resolves every root of
the document (parse, registry lookup, template construction) before any
spawning; an invalid document therefore never touches the world.

**Apply tracks what it spawns.** `ResolvedScene::apply` records every entity
it creates (recursively, in spawn order) and, on any error, despawns them in
reverse order before returning — children before parents, so the `ChildOf`
unlink hooks never meet a dead target. `load_scene` adds the symmetric
guarantee across roots: a root whose apply fails causes the roots already
spawned by that call to be despawned (a hierarchy root's linked children
cascade).

**The guarantee is bounded to operation-owned entities.** Templates build
through `TemplateContext`, which hands them `&mut World`: a template can load
assets, mutate resources, or spawn its own entities, and none of that is
rolled back. Assets loaded into the world's asset storage stay cached — the
asset server's path cache makes a retry serve them without re-reading.

## Alternatives considered

- **A fully separated two-phase commit** (build every template output against
  the world, then spawn everything). Deferred: `Template::build` legitimately
  needs `&mut World` for asset resolution, and erasing the built outputs for
  a second phase would require restricting exactly the side effects the
  template interface exists to allow. The proposal itself scopes strong
  atomicity behind restricting those effects.
- **Despawning only the failed subtree's root.** Lost: `apply` links children
  to their parent only after each child fully applies, so a mid-apply failure
  leaves entities that no root reaches; tracked cleanup is what makes the
  guarantee cover them.

## Consequences

- Invalid later roots, missing assets, and failed child templates leave no
  operation-owned entities behind; the behavior is pinned by tests in
  `moonfield-scene` (`resolved.rs` for the apply guarantee, `file.rs` for
  whole-document loads, including asset-cache retention on failure).
- Retrying a failed `load_scene` into the same world is safe: entities never
  duplicate, and already-loaded assets are served from the path cache.
- Callers that need stronger atomicity (resource rollback, template-spawned
  entities) do not have it; that requires a restricted template side-effect
  contract, not more cleanup.

# Agent Note: Editor action interface

Status: implemented

[中文](2026-10-14-editor-action-interface.zh.md)

## Problem

The [architecture contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)'s
sixth item: editor UI callbacks executed asset loading, scene I/O, and
Transform edits directly against `World`, so undo, background imports, and
automation would each reimplement the same operation, and no operation had
a validation or result contract.

## Decision

**`EditorAction` is the single execution interface.** `moonfield-editor`'s
`action` module owns validation, results, and the undo record. `execute`
runs one action and returns an `ActionOutcome` (status message, affected
entity, inverse action); `perform` executes and records the inverse in the
`EditorHistory` world resource; `record` turns an already-applied reversible
gesture into one undo unit. `ImportAsset`, `LoadScene`, `SaveScene`,
`SetTransform`, `DeleteEntity`, and `Batch` are the operations; `Batch`
rolls back its completed steps when a later step fails.

**Every UI path routes through it.** The content and scene panels call
`perform`; Ctrl/Cmd+Z and Ctrl/Cmd+Shift+Z (or Ctrl/Cmd+Y) walk the
history. A gizmo drag writes its per-frame increments directly for
immediate feedback, then records one undo unit (pre-drag transform → end
transform) when the gesture ends. The render-frame handoff stays outside
the interface.

**Windowless tests execute the same operations.** The `action` unit tests
drive `perform`/`undo`/`redo` without a display: import and scene load are
reversible, failed imports record nothing, `SetTransform` validates its
entity and component, batches roll back, and a recorded gesture undoes to
its start state.

## Alternatives considered

- **Undo at the `World` level.** Lost: component churn has no meaningful
  inverse once an entity is despawned, and a generic undo layer cannot name
  operations like "load scene". The action layer owns the operation and its
  inverse together.
- **One undo unit per drag frame.** Lost: a single drag would flood the
  history with near-identical entries; `record` coalesces the gesture into
  one unit whose inverse is the pre-drag transform.
- **A command queue with deferred application.** Deferred: nothing queues
  actions across frames yet. Background imports would build on this
  interface, and a queue designed before those consumers would guess their
  requirements.

## Consequences

- UI code owns presentation only (status text, selection); every mutation
  an undo needs goes through the action interface.
- `DeleteEntity` is deliberately not reversible — despawn discards
  components, so an inverse would have to snapshot and respawn the subtree.
- The inspector's reflect-driven editing writes components directly; undo
  covers exactly the operations above.

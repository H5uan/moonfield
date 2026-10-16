# Agent Note: Editor action interface

Status: implemented

[中文](2026-10-14-editor-action-interface.zh.md)

## Problem

The [contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
requires one execution interface for UI, automation, and undo. Recording an
inverse against a spawned entity requires refreshing that inverse when redo
creates a new entity generation. A batch cannot promise rollback while
accepting deletion or file writes that have no compensation.

## Decision

**User actions and compensation have separate types.** `EditorAction`
expresses import, scene load/save, transform and reflected-field edits,
deletion, and reversible batches. `EditorUndo` expresses restoring values, removing created
roots, and ordered groups of compensation. `ActionOutcome` returns the
affected entity, status, and optional compensation. `perform` records a
successful action; `record` coalesces a completed gizmo gesture.

**Batches validate reversibility before execution.** Nested `DeleteEntity`
and `SaveScene` actions are rejected before any step runs. A failed
reversible batch applies completed steps' compensation in reverse order and
collects all compensation errors. `ActionError::Rollback` preserves
both the original failure and these errors. Scene templates and component
hooks can have unrelated side effects; the guarantee excludes those effects
and assets retained in the load cache. Compensation validates all referenced
entities and components before mutation.

**History records the identities created by each execution.** Redo replaces
the stored compensation with its successful outcome, rather than retaining
the previous entities' generations. Failed undo or redo keeps its entry on
the original stack for retry. New reversible edits and explicit deletion
clear redo; saving a file does not discard redo.

Discrete UI operations call `perform`; keyboard shortcuts call `undo` and
`redo`. Gizmo increments write directly for same-frame feedback and
record one compensation when the gesture ends.
[Inspector field transactions](../bug-fix/2026-10-16-inspector-edit-transactions.md)
use the same history. The render-frame handoff remains outside this interface.

## Alternatives considered

**Undo at the World level.** Rejected: arbitrary component deletion has no
typed inverse, and world mutations do not identify user operations.

**Use user-action batches for compensation.** Rejected: removing imported
entities is valid compensation, but admitting deletion into a user batch
makes its rollback promise false. Separate types encode that distinction.

**Keep partial failures implicit.** Rejected: an original error hides a
failed rollback and leaves callers unable to distinguish restored state from
partial state. Compensation errors are part of the result.

**One undo unit per drag frame.** Rejected: coalescing a gesture preserves
the user's operation without flooding history with intermediate increments.

## Consequences

- Import and scene load support repeated undo/redo using the current entity
  generations, and rejected history operations remain retryable.
- Deletion and saving execute as standalone actions, not reversible batches.
- Windowless tests use the same execution and compensation interfaces as UI.
- Deletion discards arbitrary components and is not reversible. Hook and template side effects have
  no general rollback mechanism.

# Agent Note: Inspector reflected-field transactions

Status: implemented

[中文](2026-10-16-inspector-edit-transactions.zh.md)

## Problem

Inspector drawing acquired mutable component references even without widget
changes. Dereferencing those references marked components `Changed`, causing
change-filtered systems to process idle selections. Reflected field writes
also lacked user-operation boundaries and undo compensation.

## Decision

The Inspector renders from shared component references and edits owned leaf
drafts. Supported field values and concrete component-access callbacks live
in the editor, not in the reflection trait. Actions resolve named-field paths
on demand and validate the target and value type before mutable ECS access.
Undo retains field snapshots rather than entire components.

Continuous edits use the existing editor history with one compensation for
the gesture. Pending edits finish before other history operations, preserving
their execution order. The runtime contract is in
[architecture.md](../../../../docs/architecture.md#reflection-and-the-inspector).

## Alternatives considered

**Clone and compare entire components.** Rejected: registration would need
extra cloning and equality bounds, and undo could overwrite unrelated or
ignored fields. Leaf snapshots work with non-clone components.

**Bypass ECS change detection during drawing.** Rejected: the editor would
need a separate change-marking path and trust widget reports after mutating
the component. Shared drawing keeps idle UI outside mutable ECS access.

## Consequences

Reflection stays a named-field access interface without dynamic cloning or
serialization. The editor's owned value set follows its supported widgets;
other leaves remain unsupported. String widgets require an owned draft even
on idle frames; numeric and math drafts are copied values.

History validates current entity generations, component presence, field
paths, and leaf types instead of retaining ECS references. Component and
field removal can make an entry temporarily inapplicable; failed history
operations keep the entry for retry. Edits outside editor actions and
component hook side effects have no general history or rollback mechanism.

Windowless tests cover idle drawing, actual egui input, nested non-clone
components, unchanged values, stale targets, batch validation, and gesture
completion and history ordering.

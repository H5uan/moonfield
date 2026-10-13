# Agent Note: Buffered messages (Message/Messages/reader/writer params)

Status: implemented

[中文](2026-08-19-buffered-messages.zh.md)

## Problem

The engine needed the reference implementation's buffered-event mechanism
(renamed in its current dev branch to *messages*): a channel where writers
push values that multiple systems can each consume once, with automatic
cleanup — replacing the hand-rolled frame-scoped queues (`WindowEvents`,
`RawWindowEvents`) that the windowing backend and editor cleared manually at
every frame boundary. The roadmap explicitly made the `WindowEvents` channel
the migration target for this mechanism.

## Decision

`moonfield-ecs` gains `message.rs`, an architecture-level port of the
reference's `bevy_ecs::message`:

- `Message` is a blanket-implemented marker trait (`Send + Sync + 'static`),
  consistent with `Component`/`Resource` here — no derive in this workspace.
- `Messages<M>` is the double-buffered store resource: `write` appends to the
  current buffer with a monotonically increasing `MessageId`; `update()`
  swaps the buffers and clears the oldest, giving every message a two-frame
  lifetime (read once per frame → never drop; skip a frame → maybe drop;
  skip two → old messages are gone).
- `MessageCursor<M>` is per-reader state; `MessageReader<M>` /
  `MessageWriter<M>` are system params (the reader's cursor is its
  `SystemParam::State`).
- `App::add_message::<M>()` inserts the resource and registers the type in
  the `MessageRegistry` resource; `message_update_system` (exclusive system)
  swaps all registered buffers once per frame in the new `First` schedule,
  which `App::update` runs before `Update`.

Migration: `WindowEvents` and `RawWindowEvents` are deleted. The winit
backend writes `WindowEventKind` and raw winit `WindowEvent`s into
`Messages<…>` resources; the editor (an exclusive render system) keeps a
`MessageCursor<WindowEvent>` in its `EditorState` and drains new raw events
into egui. `InputState` stays latched (the reference likewise keeps
button-input state separate from its message streams); its internal replay
queue was **not** migrated — nothing outside its own tests consumes it, so
moving it would be churn without a consumer.

Minimal-port deviations, documented in the module docs: the reference's
change-tick-based skipping of unchanged buffers and its fixed-update
signaling are not ported (our resources carry no per-resource change ticks);
buffers swap unconditionally, which is observationally identical for
per-frame readers.

## Alternatives considered

- **Keep the frame-scoped queues alongside messages.** Rejected: two
  overlapping event mechanisms invite drift; the whole point was to replace
  the hand-rolled clearing pattern.
- **Migrate `InputState`'s internal event queue too.** Rejected for now: no
  consumer reads it, and the latched pressed/just-pressed semantics are a
  different contract than a message stream. Revisit if a gameplay consumer
  appears.
- **Type-erased per-resource change-tick tracking to skip unchanged buffer
  swaps.** Rejected as premature: the swap is a `Vec::clear` per registered
  type per frame; the reference's optimization exists for its parallel
  executor world.

## Consequences

- Any event-like channel can now be one `app.add_message::<T>()` call away;
  readers get per-system cursors for free.
- Message params panic on fetch if the type was never registered (same
  policy as `Res<T>` on a missing resource); the panic message names
  `App::add_message`.
- `App::update` now runs `First` before `Update`; user systems may also be
  scheduled into `First`.
- Messages are not drained at frame end anymore: a reader that never runs
  leaves at most two frames of messages buffered, then they drop silently —
  same semantics as the reference.

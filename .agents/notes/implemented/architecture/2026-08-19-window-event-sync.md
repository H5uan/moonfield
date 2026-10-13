# Agent Note: Window event synchronization

Status: implemented

[中文](2026-08-19-window-event-sync.zh.md)

## Problem

Two directions of window state flow need synchronization: winit reports
resize/DPI/focus changes, and gameplay/editor code mutates window properties
like title and cursor mode. Without an agreed-on model, each client would need
its own copy of "what the window looks like", drift between the component and
the native window would accumulate, and a request queue would race with the
component state it is supposed to drive.

## Decision

Windows are ECS entities, and the `Window` component is the single source of
truth for logical window state in `moonfield-window`.

- **winit→ECS is immediate**: winit's `window_event` writes resize/DPI/focus
  back into the `Window` component on the spot.
- **ECS→winit is diffed once per frame**: after `App::update`,
  `sync_windows` (`windows.rs`) diffs live `Window` fields against a
  `CachedWindow` component, and `diff_window` returns a `WindowDiff`
  (title, cursor mode) that the backend applies to the native window. This is
  the `CachedWindow` diff pattern without change detection.
- `WinitWindows` (resource) maps `Entity ↔ WindowId`; the primary window
  entity is spawned in `resumed`, adopting a pre-created `Window` entity if one
  exists.
- There is **no `WindowRequests` channel** — mutate the component.
- Lifecycle events (`close_requested`/`resized`/`focus_*`/`scale_factor_changed`)
  travel on a message channel — the `Messages<WindowEventKind>` resource
  (see [Buffered messages](../feature/2026-08-19-buffered-messages.md)) — so
  they are never missed by a consumer that does not poll every frame.
- Exit policy mirrors the `auto_accept_quit` convention: `CloseRequested` exits
  by default; `WindowControl::set_auto_exit_on_close(false)` hands control over,
  and `WindowControl::request_exit()` exits later.

## Alternatives considered

- **A `WindowRequests` channel (queue of mutations).** Rejected: it creates a
  second source of truth that can diverge from the component, adds a commit
  point, and is natural to accidentally bypass. Mutating the component and
  diffing once per frame keeps one truth with bounded per-frame cost.
- **Push every field every frame.** Rejected: applying unchanged values
  (cursor mode, title) costs winit calls per frame and makes the diff invisible
  to tests; the per-field diff keeps the surface intentional.
- **Track changes with change detection.** Rejected: the component is shared
  with the editor and read by many systems; a `CachedWindow` diff is simpler
  than coupling `Window` mutations to the ECS change-tick machinery, and the
  diff itself is the testable unit.

## Consequences

- Anyone changing window state must mutate the `Window` component — there is no
  other door, which makes wrong usage easy to spot.
- The diff runs once per frame, so rapid changes collapse to the final value:
  intentional, but worth remembering when testing cursor/title behavior in
  bursts.
- `WindowEventKind` messages persist for two frames; consumers read them with
  a per-reader cursor, so there is nothing to drain manually at frame end.
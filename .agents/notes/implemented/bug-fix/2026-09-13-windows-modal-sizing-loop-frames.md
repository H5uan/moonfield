# Agent Note: Driving frames through the Windows modal sizing loop

Status: implemented

[中文](2026-09-13-windows-modal-sizing-loop-frames.zh.md)

## Problem

Dragging a window edge on Windows showed no rendered content for the whole
drag — the window stayed blank (compositor background) until the mouse was
released. The frame loop is redraw-driven: `about_to_wait` requests redraws
and `WindowEvent::RedrawRequested` runs the tick via `App::update`. While
the user sizes or moves a window, Windows runs a modal message loop inside
`DefWindowProc` (`WM_ENTERSIZEMOVE`…`WM_EXITSIZEMOVE`); winit's pump never
reaches `about_to_wait`, and — contrary to what winit's
`CS_HREDRAW | CS_VREDRAW` class style suggests — `WM_PAINT` is not
dispatched inside the modal loop either, so `RedrawRequested` stops
arriving.

Measured with a programmatic repro (posting `WM_SYSCOMMAND(SC_SIZE)` plus
keyboard input enters the real modal loop): during a ~1.8 s modal sizing
loop the editor ran ~15 frames total instead of ~108, with
`about_to_wait`/`RedrawRequested` at +2/s against the usual +60/s. The
[swapchain retirement work](../architecture/2026-09-13-deferred-swapchain-retirement.md)
had already made per-tick recreation cheap; the missing piece was that no
tick ran at all.

## Decision

The classic `SetTimer` remedy, contained in a Windows-only module
(`crates/moonfield-winit/src/sizing.rs`):

- `resumed` subclasses each winit window (`SetWindowSubclass`). The
  subclass proc starts a 16 ms window timer on `WM_ENTERSIZEMOVE`, stops it
  on `WM_EXITSIZEMOVE`, and tears itself down on `WM_NCDESTROY`; everything
  else forwards to `DefSubclassProc` (winit's proc, which keeps its own
  `MARKER_IN_SIZE_MOVE` bookkeeping intact).
- The modal loop dispatches `WM_TIMER`, so each timer tick runs one frame.
  The frame goes through the *same* entry point as normal frames:
  `WinitHandler::run_modal_frame` calls `self.app.update()` exactly like
  `run_frame` does, and performs the same `last_frame`/`redraw_pending`
  bookkeeping. Only the exit-request check is deferred to the next
  `about_to_wait` (no `ActiveEventLoop` is reachable from the timer).
- The subclass proc reaches the handler through a thread-local type-erased
  driver (`*mut ()` context + shim fn) registered by `winit_run` for the
  event loop's duration. Everything involved — the event loop, the
  subclass proc, the timer — lives on the one event-loop thread, and the
  timer only fires while that thread is blocked inside the modal loop's
  pump, where no frame is on the stack, so the re-entrancy cannot overlap
  a running frame.
- `moonfield-winit` gains a `cfg(windows)`-only `windows-sys` dependency
  (same 0.61 line winit already builds; the crate is the designated OS
  binding layer).

Verified with the same programmatic modal repro: with the fix, the modal
period drives `run_frame` at +42..+46/s (timer cadence minus frame cost)
while `about_to_wait` stays at +2/s — and with the editor's
`MOONFIELD_EDITOR_SIM_RESIZE` hook forcing a swapchain recreate every
tick, the whole modal period ran recreate + present per frame with zero
errors.

## Alternatives considered

- **Rely on `WM_PAINT` re-entrant dispatch.** winit dispatches
  `RedrawRequested` synchronously from its `WM_PAINT` handler, but the
  measurement shows `WM_PAINT` is not delivered during the modal loop at
  all, so there is nothing to rely on.
- **winit's `with_msg_hook`.** The hook sees messages retrieved by winit's
  own pump; the modal loop pumps internally inside `DefWindowProc`, and
  `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` are *sent* to the window proc, never
  queued — the hook cannot observe any of the three messages this fix
  needs.
- **Drive frames from a worker thread via `EventLoopProxy`.** User events
  are processed by winit's pump, which is exactly what is blocked; the
  proxy wakes nothing until the modal loop exits.
- **A dedicated render thread.** Decouples rendering from the modal loop
  but splits the `App`/world across threads — the ECS and the whole frame
  loop are single-threaded by design. Far more invasive than a timer.
- **Always-on staleness timer (no subclass).** A timer that drives a frame
  whenever the loop looks idle cannot distinguish a modal block from a
  Reactive-mode idle without false positives; keying the timer strictly to
  `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` is exact and mode-independent.

## Consequences

- Dragging or moving the editor window on Windows keeps rendering at the
  timer cadence (~40–60 fps depending on frame cost); content tracks the
  window size live because each timer frame runs the full update →
  recreate → present path.
- `WM_ENTERSIZEMOVE` also covers title-bar moves and the system menu; those
  keep rendering too, which is the desired behavior.
- The mechanism is inert outside modal loops: the timer exists only between
  the enter/exit messages, so normal-mode pacing (`WinitSettings`,
  Continuous/Reactive) is untouched.
- The programmatic modal repro used for measurement (posted
  `WM_SYSCOMMAND(SC_SIZE)` + keyboard input) was removed after use; the
  editor keeps the `MOONFIELD_EDITOR_SIM_RESIZE` hook, which covers the
  recreate path but cannot reproduce the modal block — a real drag remains
  the manual regression check.

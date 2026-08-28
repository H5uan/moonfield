# Agent Note: Timeline semaphore frame loop

Status: implemented

[中文](2026-08-28-timeline-frame-loop.zh.md)

## Problem

Each window's frame loop throttled with a fence pool: `MAX_FRAMES_IN_FLIGHT`
per-slot `Fence`s that needed an explicit `wait` + `reset` every acquire, plus
two per-slot binary semaphore sets. The fence is a two-state latch; frame
pacing and slot reuse want a monotonic counter.

## Decision

`WindowSurfaceData` replaces the in-flight fence pool with one timeline
semaphore (`Semaphore::new_timeline(&device, 0)`) and a `frame_submitted`
counter starting at 1 — the reference project's (`no_gfx_api`) `frame_sem`
shape:

- Frame `n` uses slot `(n-1) % MAX_FRAMES_IN_FLIGHT`; before acquire (and
  before `acquire_next_image` re-signals that slot's binary `image_available`)
  the loop waits `frame_submitted - MAX_FRAMES_IN_FLIGHT` on the timeline.
- The submit path is `Device::submit_frame_timeline` using
  `vkQueueSubmit2` (`SubmitInfo2`): waits the binary acquire signal at the
  color-attachment stage, signals the binary present semaphore and the
  timeline with value = the current frame number, fence-free. Timeline values
  are strictly increasing, so no reset exists anywhere in the cycle.
- `image_available` / `render_finished` stay binary: `vkAcquireNextImageKHR`
  and `vkQueuePresentKHR` both require binary semaphores, so the present flow
  is untouched.

`Fence` remains in the RHI (other paths still use it) but the frame loop no
longer does.

## Alternatives considered

**Bridge the timeline to a single binary semaphore for present** (record a
one-command buffer that waits the timeline value and signals the binary, as
the reference does). Rejected: saves two binary semaphores per window at the
cost of an extra per-frame submit; the present path already names a binary
semaphore anyway.

## Consequences

- Frame pacing is one counter: `wait(n - MAX_FRAMES_IN_FLIGHT)` before
  starting frame `n`, signal `n` on submit. No fence wait/reset per frame.
- Swapchain recreation (`device.wait_idle`) needs no timeline reset; the
  counter just keeps counting.
- The same counter is the frame signal the upload path's arena reclaim hangs
  on (Phase 1, next step).
- Steering: acquiring before the previous cycle of the slot finished would
  double-signal a binary `image_available`; the timeline wait always precedes
  `acquire_next_image`.
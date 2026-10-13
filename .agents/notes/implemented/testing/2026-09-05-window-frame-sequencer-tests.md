# Agent Note: Window frame sequencer extracted for GPU-free tests

Status: implemented

[中文](2026-09-05-window-frame-sequencer-tests.zh.md)

## Problem

`moonfield-render-core`'s `WindowSurfaceData::acquire`/`submit` carry the
window frame loop's integer state machine — the frame-slot computation
`(frame_submitted - 1) % MAX_FRAMES_IN_FLIGHT`, the timeline wait value
`frame_submitted - MAX_FRAMES_IN_FLIGHT`, the timeline signal value, the
in-progress flag, and the recreate flag — interleaved with Vulkan calls, so
none of it was testable without a GPU. The workspace's GPU tests skip on
machines without a compatible driver, leaving this state machine with zero
executed coverage anywhere.

## Decision

The sequencing state and arithmetic now live in a plain-data
`FrameSequencer` inside `window.rs`: `plan_acquire` (slot + wait value, or
`None` on double acquire), `note_acquired`, `note_out_of_date`,
`note_recreated`, `take_for_submit` (image + slot + signal value),
`finish_submit`, and read accessors. `WindowSurfaceData` keeps the Vulkan
calls and interleaves them between the sequencer's transitions, unchanged in
order and side effects — a faithful extraction, not a redesign.

Seven unit tests cover the arithmetic exhaustively: ring fill without waits,
wait values once the ring wraps, slot cycling, signal == frame number, double
acquire rejection, out-of-date abort leaving counters untouched,
suboptimal/recreate flag lifecycle, and the submit-without-acquire panic
contract.

## Alternatives considered

- **Mock the swapchain/device behind traits and test `WindowSurfaceData`
  directly.** Rejected: the RHI's handle types are concrete `ash` wrappers
  with no trait seam, and building one for a test harness would add an
  abstraction the production code does not need.
- **Cover it through editor integration tests.** Rejected: frame-boundary
  races and off-by-one waits essentially never fire in a demo that presents
  successfully; the failure modes here are silent corruption, not crashes.
- **Wait for GPU-capable CI.** Rejected: the arithmetic needs no GPU at all;
  coupling its coverage to driver availability conflates two separate
  problems (CI has no compatible GPU — that stands).

## Consequences

- `cargo test -p moonfield-render-core` now exercises the frame-loop
  sequencing on any machine, GPU or not (7 new tests, 11 total).
- The extraction surfaced a pre-existing anomaly, since fixed: when
  `queue_present` failed with a hard error (not `SurfaceOutOfDate`), the
  timeline was already signaled but `frame_submitted` did not advance, so
  the next `submit` would have re-signaled the same timeline value — invalid
  for timeline semaphores. `submit` now calls `finish_submit` immediately
  after the queue submission; `presented_frames` counts submitted frames
  (present may still have failed).
- `FrameSequencer` is private to `window.rs`; no public API changed.

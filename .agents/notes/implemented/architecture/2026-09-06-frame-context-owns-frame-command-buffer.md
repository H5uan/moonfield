# Agent Note: FrameContext owns the frame command buffer

Status: implemented

[中文](2026-09-06-frame-context-owns-frame-command-buffer.zh.md)

## Problem

The frame's command buffer lived per window in `WindowSurfaceData`. Pass
systems fished "any in-progress window" out of the `WindowSurfaces` map to
record offscreen passes (`values_mut().next()` /
`find_map(current_command_buffer)`), with four consequences: offscreen and
viewport passes silently rendered nothing when no window frame was in
progress; offscreen work was submitted on whichever window happened to
acquire; the frame slot had two authorities (the per-window `FrameSequencer`
vs `FrameDrawArena`'s `current`); and `device.begin_gpu_frame(slot)` — the
retirement-ring drain — ran once per window per frame, which breaks the
moment a second window exists.

## Decision

A render-world `FrameContext` resource (moonfield-render-core) owns the
frame: the command pool, the per-slot command buffer ring, the timeline
semaphore, and the `FrameSequencer` state machine (frame number, slot,
in-progress flag — the machine is unchanged, only the per-window image
tracking left it). `WindowSurfaceData` keeps per-window state only: surface,
swapchain, depth, the binary `image_available`/`render_finished` sets, the
acquired image index, and the recreate flag as plain fields.

- `acquire_window_frames` begins the frame every `Render` tick a device
  exists: timeline wait, one `device.begin_gpu_frame(slot)`, command buffer
  begin, descriptor heap bind. `WindowFrameDemand` gates swapchain
  acquire/present only, not frame existence.
- Pass systems read the slot and command buffer from `FrameContext`;
  `FrameDrawArena::begin_frame(slot)` takes the slot from the frame, so the
  slot has one authority. Offscreen passes record unconditionally;
  window-targeted views record per in-progress surface into the same frame
  command buffer.
- `submit_window_frames` submits once: it waits every acquired window's
  `image_available[slot]`, signals every `render_finished[slot]` plus the
  timeline with the frame number, then presents each acquired window. A
  frame with no acquired window submits timeline-only.
- `Device::submit_frame_timeline` takes `&[&Semaphore]` slices for waits and
  signals.

## Alternatives considered

- **Per-window ownership (the old design).** Rejected: the frame is a
  device-level concept — one command buffer, one timeline, one retirement
  drain — and per-window ownership made offscreen rendering depend on an
  unrelated window's state, put the submission on an arbitrary window, and
  scaled the retirement drain with the window count.
- **A separate offscreen submit queue.** Rejected: a second submission
  stream splits the timeline and retirement-ring accounting in two, and
  ordering offscreen work against window passes (shared descriptor heaps,
  shared view targets) becomes manual; one command buffer per frame keeps
  the execution order identical to the recording order.

## Consequences

- Offscreen/viewport passes render every tick a device exists, with or
  without a window.
- `begin_gpu_frame` runs exactly once per frame; a second window no longer
  breaks the retirement ring.
- Windows that minimize or lose demand stop acquiring/presenting while the
  frame loop and offscreen rendering continue.
- The presented-frame counter is frame-level (`FrameContext`); the editor's
  feedback channel reads it from there.

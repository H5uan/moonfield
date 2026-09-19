# Agent Note: Offscreen transitions via the shared uploader; sort scratch is device-local

Status: implemented

[中文](2026-09-19-offscreen-transition-and-sort-scratch-memory.zh.md)

## Problem

Two render-stack performance defects from a code audit:

1. Every `OffscreenTarget` creation and resize ran a blocking layout
   transition: a one-shot command pool, command buffer, and fence, then
   `wait_for_fences(u64::MAX)` on the calling thread. The caller is
   `prepare_view_targets` in the `PrepareViews` set, so dragging the editor
   viewport stalled the graphics queue every frame.
2. `RadixSort`'s scratch buffers (`tmp_keys`/`tmp_values`/`hist`/`offsets`)
   were allocated with `Memory::Default` (CpuToGpu): host-visible memory the
   scatter pass reads and writes several times per sort but the CPU never
   touches.

## Decision

- `OffscreenTarget` records the fresh image's `UNDEFINED` → `GENERAL`
  transition into the device's shared `FrameUploader`
  (`Device::uploader` → `FrameUploader::transition_image`, the path built
  for storage images) instead of submitting and waiting in place. Ordering:
  `FrameContext::end_frame` flushes the uploader and submits the frame
  command buffer with a timeline wait on the uploader's latest batch at
  `ALL_COMMANDS` (`Device::submit_frame_timeline`), so the transition
  completes before any frame command — both the pass rendering into the
  target and the egui sampling of it. Callers outside a frame loop (gpu
  tests, readback tests) flush the uploader themselves before submitting;
  `Device::submit_and_wait` already waits on the uploader's latest batch.
- The four scratch allocations moved to `Memory::Gpu` (device-local). Only
  the caller's `keys_in`/`values_in`/`keys_out`/`values_out` stay
  host-visible — the host writes the inputs and reads the outputs.
- `Fence::raw` lost its only caller and was removed.

## Alternatives considered

- **Record the transition into the frame command buffer.** `prepare_view_targets`
  is a prepare system with no `RenderContext` door, so the frame's command
  buffer would have to be threaded down to it; the uploader path needs no
  plumbing and already batches this frame's other transitions and uploads.
- **Flush the uploader inside `OffscreenTarget::create`/`resize`.** An
  `end_frame` there submits without blocking and would keep test callers
  unchanged, but it splits the frame's upload batch per target and makes a
  creation API submit GPU work as a side effect. An explicit flush at the
  frame boundary (and in tests) matches how `Texture::bindless` callers
  already work.
- **Keep `Memory::Default` for the sort scratch.** Buys nothing — no code
  path maps the buffers — and spends host-visible memory bandwidth on pure
  GPU scratch.

## Consequences

- Viewport creation and resize cost one recorded barrier in an
  already-recording batch; the render thread never waits on the queue there.
- Headless and test callers of `OffscreenTarget` own one extra step: flush
  `Device::uploader()` before their own submissions (the editor's egui
  headless test already did; the rhi gpu tests and the core-3d pass test
  gained the flush).
- The radix sort's determinism acceptance test passes unchanged with
  device-local scratch.

# Agent Note: Frame-scoped uploader with per-slot arenas

Status: implemented

[中文](2026-08-28-frame-uploader.zh.md)

## Problem

Uploading into a `GpuOnly` (device-local) buffer meant creating a staging
buffer, a command pool, and a one-shot command buffer **per call**, then
blocking on `queue_wait_idle`. The staging side is temporary data with a
frame-bound lifetime, which the bump arena already models; the blocking is
per-upload, not per-frame.

## Decision

`moonfield-rhi`'s `vulkan/upload.rs` owns `FrameUploader<'a>` — the reference
project's per-frame arena shape wired to a timeline:

- `UPLOAD_FRAME_RING` slots, each holding a `GpuBumpAllocator` **and its own
  command buffer**. `begin_frame` waits `next_frame - RING` on its timeline,
  then `free_all`s the slot's arena and re-records the slot's buffer; copies
  are appended with `upload`; `end_frame` submits once signalling the
  timeline with the frame number.
- The reclaim of a slot is exactly the signal that its arena and command
  buffer are safe to reuse — **arena and command buffer must share the same
  reuse cycle** (`wait(n - RING)`). Re-recording a `ONE_TIME_SUBMIT` buffer
  still executing is undefined, so a single shared buffer cannot serve a
  ring that overruns by one frame.
- `upload` accepts only `GpuOnly` destinations and stages via
  `BumpAlloc` (`cpu` for the memcpy, `src`/`src_offset` for the copy
  command). Host-visible destinations are written directly by the caller
  and rejected here.
- `upload_and_wait` is the one-shot load-time path: begin, upload, end,
  wait.

`VK_KHR_surface` joins `REQUIRED_DEVICE_EXTENSIONS`: it is
`VK_KHR_swapchain`'s required extension and validation (VUID 01387) rejects
enabling the swapchain without it; the error cascaded into spurious
feature/allocator VUIDs.

## Alternatives considered

**One shared command buffer across all slots.** Rejected in practice: the
ring lets frame `n` start while `n-1` is still executing only if the slot
resources match that pacing; a single buffer forces `wait(n - 1)` and
serializes the pipeline.

## Consequences

- Many uploads in one frame = one submit; staging and command objects are
  created once per uploader, not once per upload.
- Field order in `FrameUploader` matters: `cb` (which calls
  `vkFreeCommandBuffers` on drop) is declared before `pool` (whose drop
  destroys the pool and frees any live buffers) — struct fields drop in
  declaration order, unlike locals.
- Validation-clean device creation: the swapchain extension dependency is
  explicit.
- Consumers (`Buffer::upload`, `Texture::upload`, the editor's egui texture
  path) move to this uploader next; the headless tests drive the full frame
  cycle themselves (`upload_ring.rs`).
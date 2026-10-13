# Agent Note: Pipeline retirement and upload→frame memory visibility

Status: implemented

[中文](2026-09-19-pipeline-retirement-and-upload-visibility.zh.md)

## Problem

Three Vulkan lifetime and synchronization defects around the frame loop:

1. `GraphicsPipeline::drop`/`ComputePipeline::drop` called
   `vkDestroyPipeline` immediately, while command buffers of up to
   `MAX_FRAMES_IN_FLIGHT` in-flight frames could still bind the pipeline.
   Reachable through shader hot reload: `Core3dPipelines::insert` drops the
   replaced pipeline mid-flight. Buffers, images, and allocations already
   retired through the device's
   [retirement ring](../architecture/2026-09-03-device-retirement-ring.md);
   pipelines did not.
2. `FrameUploader::end_frame` submits its batch with its own
   `queue_submit2`, and the frame loop submits the frame command buffer
   separately to the same queue. Submission order sequences the two batches
   but creates no memory dependency between them, so transfer writes into
   `Memory::Gpu` buffers (mesh uploads) had no barrier or semaphore making
   them visible to the frame's shader reads (device-address vertex pulls).
3. `FrameUploader::drop` never waited for in-flight batches, so its command
   pool and buffers could be freed under still-executing work at teardown.

## Decision

- Pipelines retire through the existing ring: a new `RetireAction::Pipeline`
  carries the raw device and pipeline handles, and both pipeline `Drop`s
  push it instead of destroying in place. No caller-side changes.
- Upload visibility rides the uploader's existing timeline semaphore.
  `FrameUploader::pending_signal` exposes the latest submitted value;
  `Device::submit_frame_timeline` takes a `timeline_waits` parameter, and
  `FrameContext::end_frame` flushes the uploader (the flush moved out of
  `submit_window_frames` into the frame's own submit) and passes that wait
  point. `Device::submit_and_wait` waits on the uploader's latest batch too,
  covering the test and readback paths.
- `FrameUploader::drop` waits for the last submitted batch before its
  fields (command buffers, then pool) drop.

## Alternatives considered

- **A `TRANSFER_WRITE`→`SHADER_READ` barrier at frame command buffer
  start.** A resource-less barrier would have to span all buffers, and it
  still supplies no execution ordering against the separately submitted
  upload batch. The timeline wait gives ordering and visibility in one
  mechanism the uploader already owns.
- **A fence per retired pipeline.** Multiplies sync objects for a lifetime
  question the frame-slot ring already answers; rejected by the same
  reasoning the ring note records for buffers.
- **`vkDeviceWaitIdle` in `FrameUploader::drop`.** Correct but idles
  unrelated in-flight work; the timeline wait covers exactly the uploader's
  submissions.

## Consequences

- Pipeline replacement during rendering (shader hot reload) is safe: the
  destroyed handle runs `RETIRE_RING` frames after drop, or at device
  teardown at the latest.
- Every frame submit carries at most one extra timeline wait; on a frame
  with no uploads the wait value is already signaled.
- `submit_frame_timeline` gained a parameter; the render-core frame loop is
  its only caller. The rhi public API still exposes no backend types
  (`verify_rhi_boundary.py` passes).
- `cargo test -p moonfield-rhi` (47 tests, real driver, including
  `gpu_tests::upload_ring` and `headless_triangle`) passes unchanged.

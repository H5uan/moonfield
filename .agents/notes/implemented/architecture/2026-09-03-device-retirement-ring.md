# Agent Note: Device retirement ring for deferred GPU teardown

Status: implemented

[中文](2026-09-03-device-retirement-ring.zh.md)

## Problem

Bindless resources are addressed by raw values — buffer device addresses
in push data, heap slot indices in root data — so once a frame is
submitted, nothing on the CPU side can tell whether the GPU still
references a resource. The RHI tore buffers, allocations, and images down
immediately in `Drop`, with the safety contract pushed onto callers ("the
caller defers destruction past the in-flight frames"). Every consumer had
to honor it by hand or by stalling: `OffscreenTarget::resize` idled the
whole device, the egui backend carried its own per-frame-slot deferred-free
ring, and buffer-replacement paths (bump-arena block replacement, egui
vertex-buffer growth, prepared-mesh pruning) destroyed buffers that
in-flight frames could still read.

## Decision

- `Device` owns a `RetirementRing` (inside its shared `DeviceShared` state):
  one teardown queue per frame slot, holding atomic `RetireAction`s (buffer,
  image, and pipeline destruction, heap-slot return) that resource `Drop`s
  compose.
- Covered resources — `Buffer`, `GpuAllocation`, the bump arena's
  blocks, `Texture`, `OffscreenTarget`, `DepthBuffer`, and
  `GraphicsPipeline`/`ComputePipeline` — enqueue their teardown into
  the current frame slot instead of destroying themselves.
  `Device::begin_gpu_frame` drains the slot the frame loop is about to
  record into: the in-flight timeline wait has already guaranteed that
  slot's previous submission completed. `Device::flush_retirements`
  drains every slot for tests and teardown, which must call it only with
  the GPU idle.
- Device teardown is split by ownership: `Device::drop` persists the
  pipeline cache and drops the lazy uploader/descriptor-heap singletons so
  their backing allocations retire; `DeviceShared::drop` — which runs when
  the last device referent (including any live resource) goes away — idles
  the GPU, drains the ring, frees the allocator, and destroys the device.
  All teardown runs ahead of `vkDestroyDevice`.

## Alternatives considered

- **Idling the device around destruction.** Correct but freezes the GPU;
  the resize path paid it on every viewport drag.
- **A fence per resource.** Tracks each resource individually but
  multiplies sync objects and still says nothing about heap-slot reuse
  order.
- **GPU-side reference tracking.** The untyped bindless model passes raw
  pointers and slot indices through shaders; there is no hook to count
  references on.

## Consequences

- `Buffer`, `GpuAllocation`, bump-arena block, `Texture`,
  `OffscreenTarget`, `DepthBuffer`, and pipeline teardown runs
  `RETIRE_RING` frames after drop; in-flight frames read intact memory by
  construction, and the buffer-replacement paths need no caller
  discipline. Pipelines are bound into command buffers, so a
  shader-revision rebuild mid-frame-loop retires the replaced pipeline
  the same way. `ShaderModule` stays immediate: pipelines consume the
  module at creation (the SPIR-V is baked), and command buffers never
  reference modules.
- The bump allocator carries a `DeviceContext` (the shared device-state
  handle): its block constructor is lifetime-free and cannot fetch one from
  `&Device`.
- The frame loop drives the ring: `acquire` drains the slot it is about
  to record into (after the in-flight timeline wait), and the frame submit
  flushes the shared uploader and waits on its latest submitted batch —
  the timeline wait, not same-queue submission order, is the memory
  dependency that makes upload writes visible to shader reads (see
  [pipeline retirement and upload visibility](../bug-fix/2026-09-19-pipeline-retirement-and-upload-visibility.md)).
  `RenderPlugin` asserts `MAX_FRAMES_IN_FLIGHT == RETIRE_RING`.
- Drains run outside the ring lock, and `drain_all` loops to quiescence so
  nothing queued mid-drain is left behind.
- `OffscreenTarget::resize` allocates new heap slots with the new image;
  the old slots and image retire. Heap descriptors are written once at
  creation and never rewritten, and the resize path no longer idles the
  device. Holders re-register when `texture_handle` changes — the
  editor's viewport binding refreshes on handle change.
- The egui backend's per-slot deferred-free ring is deleted: texture
  drops and frees retire through the ring, and its uploads ride the
  shared uploader.
- Device teardown order is fixed: the `Device` handle persists the
  pipeline cache and drops the lazy singletons; the shared state's drop
  (last referent) idles the GPU, drains the ring, tears down the
  allocator, and destroys the device.

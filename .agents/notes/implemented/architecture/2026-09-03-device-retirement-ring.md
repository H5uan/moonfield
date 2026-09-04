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

- `Device` owns a `RetirementRing`: one teardown queue per frame slot,
  holding atomic `RetireAction`s (buffer and image destruction, heap-slot
  return) that resource `Drop`s compose.
- Covered resources — `Buffer`, `GpuAllocation`, the bump arena's
  blocks, `Texture`, and `OffscreenTarget` — enqueue their teardown into
  the current frame slot instead of destroying themselves.
  `Device::begin_gpu_frame` drains the slot the frame loop is about to
  record into: the in-flight timeline wait has already guaranteed that
  slot's previous submission completed. `Device::flush_retirements`
  drains every slot for tests and teardown, which must call it only with
  the GPU idle.
- `Device::drop` idles the device, drops the lazy uploader and descriptor
  heap singletons so their backing allocations retire, then drains — all
  teardown now runs ahead of `vkDestroyDevice` instead of during field
  teardown after it.

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

- `Buffer`, `GpuAllocation`, bump-arena block, `Texture`, and
  `OffscreenTarget` teardown runs `RETIRE_RING` frames after drop;
  in-flight frames read intact memory by construction, and the
  buffer-replacement paths need no caller discipline.
- The bump allocator carries a `RetirementRing` handle alongside its raw
  `ash::Device` (its block constructor is lifetime-free and cannot fetch
  one from `&Device`).
- The frame loop drives the ring: `acquire` drains the slot it is about
  to record into (after the in-flight timeline wait), and
  `submit_window_frames` flushes the shared uploader ahead of the frame
  command buffers — same-queue submission order executes the uploads
  first. `RenderPlugin` asserts `MAX_FRAMES_IN_FLIGHT == RETIRE_RING`.
- Drains run outside the ring lock, and `drain_all` loops to quiescence:
  teardown can cascade, because an action releasing the last
  `Arc<DescriptorHeap>` retires the heap's backing allocations in turn.
- `OffscreenTarget::resize` allocates new heap slots with the new image;
  the old slots and image retire. Heap descriptors are written once at
  creation and never rewritten, and the resize path no longer idles the
  device. Holders re-register when `texture_handle` changes — the
  editor's viewport binding refreshes on handle change.
- The egui backend's per-slot deferred-free ring is deleted: texture
  drops and frees retire through the ring, and its uploads ride the
  shared uploader.
- Device teardown order is fixed: idle, drop the lazy singletons, drain
  the ring, tear down the allocator, destroy the device.

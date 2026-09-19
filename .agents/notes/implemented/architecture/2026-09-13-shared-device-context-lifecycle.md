# Agent Note: Shared device context for GPU object lifetimes

Status: implemented

[中文](2026-09-13-shared-device-context-lifecycle.zh.md)

## Problem

Only `GpuAllocation` kept part of the device alive (its
`Arc<Mutex<Allocator>>`); `Semaphore`/`Fence`, `Swapchain`, `CommandPool`,
both pipeline types, and `ShaderModule` held bare `ash::Device` clones. Any
of them outliving `Device` would call `destroy_*` on a destroyed device —
Vulkan UB that `Device::drop`'s allocator-based leak guard could not see.
`Surface` held no instance reference at all and survived on a field-order
convention in render-core's `WindowSurfaceData`. On top of that, the
`ash::Device + Arc<Mutex<Allocator>> + Arc<RetirementRing>` field triple was
repeated across six structs, and the "create image → query requirements →
allocate → bind → create view" sequence existed three times (texture,
offscreen color, offscreen depth).

## Decision

wgpu-style shared ownership, with the device split in two:

- `DeviceShared` (crate-internal) owns the teardown-critical state: the
  `ash::Device` handle, the `DeviceExtensionFunctions` loaders, the
  `RetirementRing`, the `Arc<Mutex<Allocator>>`, and an
  `Arc<InstanceShared>` keepalive. Its `Drop` — triggered by the last
  referent going away — idles the GPU, drains the ring, frees the
  allocator, and destroys the device. The instance Arc drops after that
  body, so the instance outlives the device by construction.
- `Device` keeps the metadata and lazy singletons (queues, queue families,
  heap properties, uploader/descriptor-heap/shader/pipeline caches). Its
  `Drop` only persists the pipeline cache and releases the singleton Arcs
  early.
- `DeviceContext` (crate-internal, `Clone`, `Deref` to `DeviceShared`) is
  the single handle every GPU object stores and every crate-internal
  constructor takes, replacing the field triples. `Semaphore`, `Fence`,
  `TimestampQueryPool`, `CommandPool`/`CommandBuffer`, both pipelines,
  `ShaderModule`, `Swapchain`, `TextureView`, `FrameUploader`,
  `GpuBumpAllocator`, `GpuAllocation`, `Texture`, `OffscreenTarget`,
  `DepthBuffer`, and `HeapSlots` all hold one.
- `Instance` wraps `Arc<InstanceShared>`; `Surface` holds an
  `Arc<InstanceShared>` keepalive, ending the field-order convention.
- `Image2d` (crate-internal, `vulkan/image.rs`) folds the three image
  creation copies into one call returning image + view + owned view create
  info + allocation.
- The retirement actions keep raw `ash::Device` clones and the allocator
  Arc — deliberately *not* `DeviceContext`: `ImageSlot` previously carried
  `Arc<DescriptorHeap>`, whose backing allocations now hold a
  `DeviceContext`, which would close a strong cycle through the ring
  (shared → ring → action → heap → allocation → shared) and leak the
  device forever. The action now holds the heap's
  `Arc<Mutex<SlotAllocator>>` directly; freeing a slot is plain CPU
  bookkeeping, valid even after the heap is gone.
- The leak guards are deleted: `Instance`'s live-device counter and
  `Device::drop`'s allocator `try_unwrap` guard guarded orderings the
  shared-ownership model makes unrepresentable. `DeviceShared::drop` keeps
  a defensive `try_unwrap` branch (leak instead of use-after-destroy) for
  the allocator, but it is unreachable by construction.

## Alternatives considered

- **One `Arc<DeviceInner>` for everything (no shared/outer split).** The
  lazy singletons (uploader, descriptor heap) live on the device and would
  hold the inner back-reference — a self-cycle that leaks the device the
  moment the uploader is initialized. The two-layer split puts only
  teardown-critical state behind the Arc, so nothing `DeviceShared` owns
  points back at it.
- **Keep the leak guards alongside the Arc model.** Dead checks on an
  impossible state mislead readers into thinking the ordering is still
  load-bearing; the keepalive comments on the fields carry the invariant
  better.
- **Weak references from singletons to the device.** Lets the singletons
  sit inside the shared state without a cycle, but every use becomes an
  upgrade that can fail, and a `FrameUploader` outliving its device could
  no longer destroy its command buffers — reintroducing the UB class this
  change removes.
- **Skip the `Image2d` helper.** The three copies differed only in usage
  flags, aspect, and allocation name, so the fold stayed clean and was
  taken.

## Consequences

- "Object outlives device" is no longer an error class: the logical device
  is destroyed when the last object using it is gone, and the instance
  outlives all devices and surfaces. Teardown-order conventions
  (`RenderDevice` field order, `WindowSurfaceData` field order) are
  documentation, not load-bearing.
- The public API is unchanged — every constructor still takes `&Device` /
  `&Instance`; downstream crates needed no edits. The boundary gate passes:
  `DeviceShared`/`DeviceContext`/`InstanceShared`/`Image2d` are all
  crate-internal.
- The field triple survives in exactly one place, by design:
  `RetireAction::{Buffer, Image}` carry raw handles plus the allocator Arc
  so ring contents never strong-reference the shared state.
- Device teardown work moved: the pipeline-cache write and singleton
  release happen in `Device::drop`; idle/drain/allocator/destroy happen in
  `DeviceShared::drop`, potentially later (and on whichever thread drops
  the last referent — all Vulkan objects are main-thread today, so this is
  the same thread in practice).
- `gpu_tests` pass unchanged, including `headless_triangle` on real
  hardware.

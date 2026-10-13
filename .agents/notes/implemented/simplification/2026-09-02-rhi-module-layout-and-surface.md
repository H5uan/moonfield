# Agent Note: RHI module layout and public surface cleanup

Status: implemented

[中文](2026-09-02-rhi-module-layout-and-surface.zh.md)

## Problem

The bindless transition left the RHI's module map and public surface
inconsistent. `bindless.rs` had become a grab-bag: memory primitives
(`Memory`/`GpuPtr`/`HostPtr`/`GpuAllocation`), barrier vocabulary
(`Stage`/`BarrierHazard`), a queue enum (`QueueType`), and the compute
pipeline all lived in one file whose name was redundant — the RHI has exactly
one resource model now, so nothing is non-bindless. `view.rs` (an
`ash`-wrapping `TextureView`) sat at crate root, outside the `src/vulkan/`
boundary the crate's AGENTS.md mandates for all Vulkan types. Several public
APIs still leaked raw `vk::` types: `bind_compute_pipeline(vk::Pipeline)`,
the `draw_indirect*` family taking `vk::Buffer`/`vk::DeviceSize`,
`GpuAllocation::buffer()`, `TextureSlotDesc` carrying a public
`vk::ImageViewCreateInfo` field, `write_buffer_descriptors` taking
`vk::DeviceAddressRangeEXT`, and the legacy `pipeline_barrier`. Meanwhile
`Device::queue()`/`QueueType` were dead — callers use
`graphics_queue()`/`compute_queue()`. One leak reached past the RHI crate:
`Buffer::new` took a `gpu_allocator::MemoryLocation`, which forced
`moonfield-render-feature` and `moonfield-editor` to depend on the
`gpu-allocator` crate directly.

## Decision

- `vulkan/bindless.rs` is renamed to `vulkan/memory.rs` and keeps only the
  memory/pointer model: `Memory`, `GpuPtr`, `HostPtr`, `GpuAllocation` — CPU
  view + device address in one object.
- `Stage` and `BarrierHazard` move to `vulkan/sync.rs`, which already owns
  `Fence`/`Semaphore`; `ComputePipeline` moves to `vulkan/pipeline.rs` next to
  `GraphicsPipeline`. `QueueType` and `Device::queue()` are deleted.
- `src/view.rs` moves to `src/vulkan/view.rs`; `TextureView` is re-exported
  through `vulkan/mod.rs` and the crate-root glob.
- No public API takes or returns a raw `vk::` type anymore, except the
  documented interop `raw()` accessors: `bind_compute_pipeline` takes
  `&ComputePipeline`, the `draw_indirect*` family takes `&Buffer` + `u64`
  offsets, `GpuAllocation::buffer()` and the legacy `pipeline_barrier` are
  `pub(crate)`, and both pipeline `raw()`s are `pub(crate)`.
- Heap write entry points go crate-internal: `TextureSlotDesc` gets
  `pub(crate)` fields and a `pub(crate)` constructor,
  `write_resource_descriptors` is `pub(crate)` (its only production writers
  are `Texture::bindless` and `OffscreenTarget`), and
  `write_buffer_descriptors` takes the new public `BufferRange { address:
  GpuPtr, size: u64 }` vocabulary instead of `vk::DeviceAddressRangeEXT`.
- `vulkan/mod.rs` re-exports the important types explicitly
  (`memory::{GpuAllocation, GpuPtr, HostPtr, Memory}`,
  `pipeline::{BlendMode, ComputePipeline, GraphicsPipeline, ShaderStageDesc}`,
  `sync::{BarrierHazard, Fence, Semaphore, Stage}`, `view::TextureView`,
  `descriptor_heap::BufferRange`), so tests and downstream crates import from
  the crate root. The descriptor-heap test's image-write case moves to the
  public `write_buffer_descriptors`; image-descriptor coverage continues in
  the end-to-end sampling tests. `docs/architecture.md`'s stale push-constant
  sentence now describes the `DrawData`/`FrameDrawArena`/`push_data` reality.
- `Buffer::new` takes the crate's own `Memory` class instead of
  `gpu_allocator::MemoryLocation` (mapped internally via
  `Memory::to_location`), `Buffer::size()` returns `u64`, and
  `Buffer::location()` becomes `Buffer::memory()`. `moonfield-render-feature`
  and `moonfield-editor` drop their direct `gpu-allocator` dependency.

## Alternatives considered

- **Keep `bindless.rs` as one module.** Rejected: the name is redundant now
  that the RHI has exactly one resource model, and the file mixed four
  concerns (memory primitives, barrier vocabulary, queue enum, compute
  pipeline) that already have natural homes.
- **Keep `TextureSlotDesc`/`write_resource_descriptors` public for tests.**
  Rejected: the only production writers are in-crate (`Texture::bindless`,
  `OffscreenTarget`), and test coverage moves to the public
  `write_buffer_descriptors` plus the existing end-to-end sampling tests.
- **Remove all `raw()` accessors (`Device`/`Instance`/`CommandBuffer`/
  `Buffer`/`Semaphore`).** Rejected: they are the load-bearing interop seam
  used by render-core's window loop and the editor's egui backend; only the
  pipeline `raw()`s tighten to `pub(crate)`.

## Consequences

- The module map matches the boundary rule: all `ash` types live under
  `src/vulkan/`, and each module names one concern (memory, sync, pipeline,
  view).
- The public API no longer leaks `vk::` types outside the documented interop
  `raw()` accessors; the `TextureSlotDesc` struct is no longer re-exported.
- The test suite's imports move to the crate root
  (`moonfield_rhi::{ComputePipeline, GpuAllocation, Memory, Stage, ...}`),
  and `indirect_draw` records through `&Buffer` instead of raw handles.
- `docs/architecture.md` no longer claims per-draw data goes through push
  constants.
- No backend crate type crosses the RHI boundary anymore: downstream crates
  (render-core, render-feature, editor) import only `moonfield_rhi`'s own
  vocabulary, and neither render-feature nor the editor depends on
  `gpu-allocator` directly.
- `texture_bindless` gains the `DEVICE_LOCK` serialization the other
  multi-device test binaries already carry: its three tests each create an
  instance + device, and doing so concurrently access-violates on some
  Windows drivers.

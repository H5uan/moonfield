# Agent Note: Metal 4 backend skeleton

Status: implemented

[中文](2026-10-09-metal-4-skeleton.zh.md)

## Problem

macOS had no GPU path: the Vulkan backend's device table cannot be satisfied
by MoltenVK, so local development on Apple silicon had no renderer at all.
The [Metal 4 backend proposal](../../proposed/architecture/2026-10-09-metal-4-backend.md)
phases the backend; its first phase is a skeleton that proves the Metal 4
surface — device gate, queue and residency, command recording with the
argument table, and a pixel-verified draw — on real hardware.

## Decision

`moonfield-rhi-metal` (under `crates/moonfield-rhi/metal/`, on
`objc2-metal`'s `MTL4*` types) ships the skeleton:

- `Instance::new_headless` acquires the system default device and enforces
  the platform gate — `supportsFamily(MTLGPUFamily::Metal4)`, a clear error
  otherwise.
- `Device` owns one `MTL4CommandQueue`, a device-wide `MTLResidencySet`
  (every buffer/texture registers itself on creation; the set lives for the
  device's lifetime), a `MTL4CommandAllocator`, and an `MTLSharedEvent`
  timeline. `submit_and_wait` is `endCommandBuffer` → queue `commit_count` →
  queue `signalEvent_value` → CPU `waitUntilSignaledValue`: the queue-level
  event signal is the fence, since `MTL4CommandBuffer` does not inherit the
  classic `waitUntilCompleted` API.
- `CommandBuffer` records on `beginCommandBufferWithAllocator` with its own
  `MTL4ArgumentTable` (8 buffer slots). Buffer binding is
  `setAddress_atIndex` — shaders see `[[buffer(index)]]`; there is no
  `setVertexBuffer` on the Metal 4 encoder, the argument table is the
  binding model. The render pass takes the core vocabulary
  (`RenderAttachment`/`RenderPassDesc`) and the first color attachment.
- `Memory` is a shared-storage `MTLBuffer` (unified memory: CPU writes via
  `contents`, GPU address via `gpuAddress`); `Texture` is a render-target +
  shader-read 2D image with CPU readback; `ShaderModule::from_msl` compiles
  MSL at runtime; `GraphicsPipeline` is vertex+fragment with one color
  format.
- The facade gains the `metal` feature and a `compile_error!` rejecting
  builds with both backend features enabled; the metal re-export list is the
  implemented subset of the Vulkan surface. `verify_rhi_boundary.py` scans
  `metal/src` with the rest.

The GPU test `metal/src/gpu_tests/offscreen_triangle.rs` renders a
vertex-pulled triangle (positions through argument-table slot 0) into a
64×64 target, submits, waits on the timeline, and verifies pixels — it
passes on Metal 4 hardware and skips with a reason elsewhere.

## Alternatives considered

**Per-command-buffer residency (`useResidencySet` on each buffer with
per-pass registration)** would scope residency to what a pass touches. It
loses against the engine's global bindless heap model, where resources are
device-lifetime and per-pass bookkeeping would be churn; the device-wide set
with registration at creation matches it.

**The classic `MTLCommandBuffer` API** (`waitUntilCompleted`,
`addCompletedHandler`) cannot back `MTL4CommandBuffer` — the Metal 4 command
buffer is a separate protocol without those methods, and the queue-level
`MTLSharedEvent` signal/wait is its synchronization model.

**Offline-compiled `.metallib` for the smoke test** would pre-shape the
Phase 2 Slang path, but requires the toolchain at build time. Runtime MSL
compilation keeps the skeleton self-contained; `.metallib` lands with the
Slang target work.

## Consequences

Apple silicon has a GPU path for local development and CI: the backend
compiles and the smoke test runs on any Metal 4 device. Downstream crates
are unchanged — every platform still selects `vulkan`; the `metal` feature
re-exports the implemented subset only, so consumers cannot reach for
unimplemented names. The skeleton's render pass takes a single color
attachment and ignores the depth attachment; `AttachmentLayout` is a
role marker with no Metal image-layout semantics. Remaining phases
(Slang `metallib`, `CAMetalLayer` swapchain, capability-gated features,
the egui port) stay with the proposal note.

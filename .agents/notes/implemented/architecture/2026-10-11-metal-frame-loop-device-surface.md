# Agent Note: Metal frame-loop device surface

Status: implemented

**Superseded:** the Metal backend is removed — see
[the Vulkan pivot note](2026-10-13-rhi-vulkan-pivot.md).

[中文](2026-10-11-metal-frame-loop-device-surface.zh.md)

## Problem

The [Metal 4 proposal](../../rejected/architecture/2026-10-09-metal-4-backend.md)'s
phase 3 delivered the `CAMetalLayer` swapchain and the platform-selected
`RenderDevice`, but the Selene window frame loop could not compile against
the Metal backend: it spells device surface the backend did not carry —
`begin_gpu_frame`, `wait_idle`, `supports_present`, the uploader's
`end_frame`/`abort_frame`, the heap's `cmd_bind`, `Swapchain::take_aborted`,
`GraphicsPipeline::id`, `OffscreenTarget`, `DepthBuffer` — and the engine
layer stores its GPU handles in ECS resources, which requires `Send +
Sync` (objc2 objects are neither by default).

## Decision

The Metal backend carries the frame loop's device surface, mapped onto
Metal's model:

- **Frame pacing and retirement.** `begin_gpu_frame(slot)` drains the
  slot's retirement queue and marks it the push target;
  `flush_retirements` drains everything. A resource a submitted command
  buffer may still reference through a raw GPU address (an argument-table
  slot, a root-blob ring) defers its final release into the ring: Metal
  command buffers do not retain address-bound resources. `Buffer`'s
  `Drop` pushes the deferred release; encoder-bound attachments and
  pipelines are retained by Metal and need no deferral.
  `wait_idle` enqueues a timeline signal after the queue's committed work
  and waits for it.
- **The uploader.** `FrameUploader::upload_alloc` stages bytes in a
  grow-on-demand shared-storage buffer and records a blit copy onto a
  classic `MTLCommandBuffer` — the Metal 4 command buffer carries no blit
  encoder in `objc2-metal` 0.3.2, so the uploader owns a classic command
  queue. `end_frame` commits the batch and waits for completion, so the
  frame submitted next observes the copies (the synchronous wait trades
  CPU-GPU overlap for ordering without cross-queue events; an
  event-ordered path replaces it if profiling demands). `abort_frame`
  drops the batch uncommitted. Host-visible allocations need no uploader:
  shared storage makes their writes direct.
- **Targets.** `OffscreenTarget` (color + optional `D32Sfloat` depth,
  views/format/extent/resize/readback) and `DepthBuffer` (window-sized
  `D32Sfloat` depth) are built from the render-target `Texture`. Their
  bindless handles stay with phase 2 — no consumer in the compiled layer
  spells them.
- **The bindless binding point.** `DescriptorHeap::cmd_bind` records
  nothing: the per-command-buffer argument table is the resource channel
  on this backend, so there is no device-global heap to bind. The
  `bindless_textures` capability stays `false` until the
  `DescriptorHandle` channel lands (phase 2).
- **Small parity items.** `Instance::supports_present` is always true
  (every Metal device presents to a `CAMetalLayer`);
  `Swapchain::take_aborted` is always false (blocking acquisition has no
  signaled-but-unconsumed state); `GraphicsPipeline::id` is the pipeline
  object's address (the bind-tracking key).
- **Ownership.** The public handles carry `Send + Sync` under the
  RHI's main-thread ownership rule (recording, submission, and resource
  use are externally synchronized by the owning frame loop — the same
  contract the Vulkan backend rests on). The impls live in one documented
  module; `CommandBuffer` already carried the same pair.

## Alternatives considered

- **A blit encoder through `msg_send!` on the Metal 4 command buffer.**
  Lost: the selector is unverified against the Metal 4 headers, and the
  classic queue is fully present in `objc2-metal` — the proposal's
  coverage-gap fallback.
- **Deferred retirement through Metal object lifetimes alone.** Lost:
  address-bound resources are exactly the ones command buffers do not
  retain; without the ring, dropping a per-frame buffer mid-flight
  leaves a dangling GPU address.
- **An event-ordered non-blocking uploader now.** Lost: cross-queue
  event ordering buys back CPU-GPU overlap for upload frames only; the
  synchronous wait is the simple correct baseline until a profile
  shows the cost.

## Consequences

- `moonfield-render-core`, `moonfield-render-feature`, and
  `moonfield-ml` compile against the Metal backend and their tests pass
  on Apple Silicon — including the frame-loop abort GPU test (acquire →
  abort → retry → submit through the full device surface) and the ml
  trainer's `gaussian_fit_converges_through_trainer` (the proposal's
  phase-4 ml autodiff→metallib verification).
- The editor's remaining Metal-path gap is the bindless channel
  (`TextureHandle`/`SamplerHandle`, `sampler_for`, `Texture::bindless`
  and `Texture::upload`, the offscreen handles) plus the egui Metal
  port — phase 2 and phase 5 of the Metal proposal.
- The engine layer's upload consumers that ride
  `Texture::upload`/`bindless` (the editor's atlas textures) land with
  that channel; `upload_alloc` serves the GPU-private static-geometry
  path render-feature spells today.

# Agent Note: RHI sync and memory contract narrowing

Status: implemented

[中文](2026-10-11-rhi-sync-memory-contract-narrowing.zh.md)

## Problem

The [RHI layering proposal](../../implemented/architecture/2026-10-10-rhi-interface-and-backend-layering.md)
requires common contracts before adapting the backends, and warns that
Vulkan-shaped contracts can force expensive or incorrect emulation on Metal.
Three contract areas sat on that fault line:

- **Barriers.** The public `Stage`/`Access` vocabulary was a Vulkan shape. On
  Metal, the `TRANSFER` and `DRAW_INDIRECT` stages collapsed or dropped in
  the `MTLStages` mapping, and the `barrier` access parameters were ignored
  entirely. A caller relying on access granularity got no guarantee on Metal
  and no error.
- **Synchronization primitives.** The public surface carried `Fence`, binary
  `Semaphore`, and `TimestampQueryPool`. On Metal the semaphore was a shell
  and waiting happened inside submission; the fence and timestamp types did
  not exist.
- **Command pools and memory.** `CommandPool` exposed queue-family selection
  and failure modes that differed per backend, and `Memory` was an
  allocation class on Vulkan but owned a buffer on Metal.

Widening Metal to honor these Vulkan-shaped contracts would have meant
emulating semantics the API does not have. The shipped direction narrows the
public contract to the semantics both backends can actually guarantee.

## Decision

The public sync, command, and memory contracts name only operations both
backends implement natively.

- **Memory residency classes, distinct from allocation objects.**
  `MemoryResidency` (`GpuOnly`, `CpuWriteOnly`, `CpuReadOnly`) is core
  vocabulary in `core/src/types.rs`, with host visibility and GPU-completion
  guarantees defined per class. `GpuAllocation` has one constructor shape
  across backends (`new`/`new_aligned` returning `Result`) and matching
  accessors (`gpu_ptr`, `host`, `read_bytes`, `size`). The Metal
  buffer-owning `Memory` type is dissolved: `GpuAllocation` owns its buffer,
  and a crate-internal `Buffer` serves the bump allocator and root ring.
  Metal maps the classes onto storage modes (shared for the host-visible
  classes, private for `GpuOnly`); Vulkan maps them onto allocator
  locations, unchanged from the old `Memory` semantics.
- **Stage-scoped barriers, no public access masks.** `Stage`
  (`VERTEX`/`FRAGMENT`/`COMPUTE`/`TRANSFER`/`ALL_GRAPHICS`/`ALL`) is core
  vocabulary; `Access` is gone. `CommandBuffer::barrier(before, after)` is
  the only barrier door. Each backend derives native stage/access pairs:
  `stage_to_vk` maps conservatively (shader stages imply
  read/write/sampled; `ALL*` imply memory read/write), `stage_to_metal`
  widens what Metal 4 cannot express to its full stage set — never narrowing
  or silently dropping. `DRAW_INDIRECT` is not in the vocabulary (no
  consumer). The texture `transition` API from the proposal did not ship:
  no pass needs explicit layout control yet, so it lands with its first
  consumer instead of ahead of one.
- **Timeline-only synchronization.** The device owns one submission
  timeline (`Timeline` with `wait`/`completed`, over a Vulkan timeline
  semaphore or the Metal shared event). `Device::submit_frame(cmd,
  acquired)` waits each acquired image's slot sync, signals its present
  sync, and signals the timeline with the frame number, returning it.
  `Swapchain::acquire` returns an `AcquiredImage`; dropping it unpresented
  is the abort (the slot's acquire semaphore is lazily rebuilt on the next
  acquire, and `take_aborted` feeds the surface's recreate flag). Binary
  semaphores live inside swapchain slots; `Fence` is deleted (no consumer
  anywhere); `TimestampQueryPool` is test-only until a capability-shaped
  consumer exists; Metal's shell `Semaphore` is deleted.
- **Queues hand out pooled command buffers.** `CommandPool` is private.
  `Device::command_buffer(usage)` returns a begun buffer from a
  device-owned pool; `begin`/`end`/`reset` are crate-internal, and the
  submission entries end buffers idempotently. The window frame loop takes
  a fresh buffer per frame instead of keeping a per-slot ring; the buffer
  frees itself back to the pool on drop.
- **Shader target and capabilities derive from backend selection.** The
  facade's `shader_target()` and `shader_capabilities()` are `const fn`s
  pinned per backend feature; consumers no longer name `Spirv` or
  `spvDescriptorHeapEXT`. Both devices own a `shader_cache()`, and
  `Device::new` has the same signature across backends (the surface
  parameter is inert on Metal).

## Alternatives considered

- **Keep access masks public; Metal errors on what it cannot honor.** Lost:
  every access constant becomes a no-op or an error on one backend, so the
  contract promises an effect the implementation cannot deliver — the
  emulation trap the layering note warns against. The highest-frequency call
  sites (the GS compute chains) need read-after-write between dispatches,
  which stage pairs already express.
- **Automatic per-resource state tracking.** Lost: bindless raw GPU pointers
  make read/write tracking inexpressible without redesigning the resource
  model, as recorded in the
  [render pass schedule redesign](../../implemented/architecture/2026-09-09-render-pass-schedule-redesign.md).
  Explicit stage pairs keep synchronization visible at the call site.
- **Keep `CommandPool` public with unified signatures.** Lost: unification
  hides queue-family selection anyway, and the only consumer pattern is a
  recordable buffer per frame. The pool is bookkeeping, not contract.
- **Keep `Fence` and binary `Semaphore` public.** Lost: no consumer outside
  swapchain and frame-pacing internals used them. `Fence` had no consumer
  anywhere and is deleted; `Semaphore` is crate-internal, and a future need
  can reintroduce a type with real semantics on both backends.
- **Public placement heaps and budget queries.** Deferred: consumers need
  exactly device-local, upload, and readback today. A placement API can land
  when a consumer exists.

## Consequences

- The facade's two backend re-export lists are now identical for every
  sync, memory, and command type; the remaining divergence is concentrated
  in capability-shaped areas (bindless descriptor heap, offscreen targets,
  timestamp queries, upload), which the layering proposal's capability work
  owns.
- `RenderContext`'s door-switch machine records stage pairs only; the
  window frame loop paces itself on the device timeline, and its sequencer
  asserts its frame counter against the device's on every submission, so a
  mismatch panics instead of desynchronizing the timeline.
- Writing the frame-abort path surfaced that the old `abort_frame` never
  satisfied its own invariant (the context was not beginnable after an
  abort); `FrameSequencer::note_abort` now defines it, and the previously
  failing window test passes.
- The contract-test suite (`crates/moonfield-rhi/tests/contract_memory.rs`)
  pins the allocation/readback/residency contract through the facade's
  public API against whichever backend the build selects; barrier ordering
  and token waits are pinned by the backends' gpu_tests. The first macOS
  run (Apple Silicon) validated the Metal halves: the full suite passes,
  including the compute contract's pending-barrier encoder replay. That
  run caught the core compiler recording the source-level entry name for
  Metal targets while Slang's Metal codegen renames `main` to `main_0`
  (warning E40100); `finish_compile` applies the rename, so a `main`-named
  source compiles on both targets. A hardware skip remains no evidence of
  equivalence — a Vulkan device carrying the required extensions must still
  run the suite.
- Stage-only barriers are as coarse as the previous global barriers, so
  over-synchronization did not get worse; if profiling shows a cost, the
  stage set widens rather than reintroducing access masks.

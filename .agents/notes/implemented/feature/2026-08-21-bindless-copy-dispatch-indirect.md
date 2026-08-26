# Agent Note: Bindless copy and indirect dispatch commands

Status: implemented

[中文](2026-08-21-bindless-copy-dispatch-indirect.zh.md)

## Problem

The bindless milestone (compute-only) shipped `GpuAllocation`, the compute
pipeline, and root-pointer dispatch, but left two commands from the original
scope unwired: GPU→GPU memory copy (`cmd_memcpy`) and reading dispatch
arguments from GPU memory (`dispatch_indirect`). The blog's `gpuMemCpy` and
`gpuDispatchIndirect` take raw GPU addresses — a shape Vulkan cannot honor
directly: `vkCmdCopyBuffer2` and `vkCmdDispatchIndirect` both require a
buffer object plus an offset, never a bare address.

## Decision

The two commands operate on `&GpuAllocation` instead of bare `GpuPtr`. The
allocation is already the address carrier from the memory-first model — it
owns the backing `vk::Buffer`, the CPU view, and the device address — so it
is the natural object for commands Vulkan forces to be buffer-object based.

- `GpuAllocation::buffer()` exposes the backing `vk::Buffer` so the command
  layer can submit it to `vkCmdCopyBuffer2` / `vkCmdDispatchIndirect`.
- `GpuAllocation::new` now creates the carrier buffer with
  `TRANSFER_SRC | TRANSFER_DST` added to `SHADER_DEVICE_ADDRESS`. The transfer
  flags are functionally required — validation rejects a copy whose source
  lacks `TRANSFER_SRC` or whose destination lacks `TRANSFER_DST` — and they
  are a static creation-time capability on a buffer, not per-resource runtime
  state tracking, so the bindless model (no resource lists, no state
  tracking) is unaffected.
- `CommandBuffer::dispatch_indirect(&GpuAllocation)` records
  `vkCmdDispatchIndirect` reading the `DispatchIndirectArgs` (x/y/z) at the
  allocation's base. Precision: args written by a prior dispatch need a
  compute→compute barrier so the command processor sees them; CPU-written
  args are visible after `queue_submit` without an explicit barrier.
  `HAZARD_DRAW_ARGUMENTS` (indirect multi-draw milestone) stays out of scope.
- `cmd_memcpy(dst, src, size)` records `vkCmdCopyBuffer2` (sync2) copying a
  whole allocation, both offsets 0. Whole-block only; sub-region copies stay
  a future need. The caller owns the transfer→consumer barrier.

Tests (`tests/bindless_memcpy_dispatch_indirect.rs`) verify both commands as
full CPU→GPU→CPU round trips on lavapipe: memcpy fills a destination with
identical values, and indirect dispatch launches the `+1` kernel with the
workgroup count read from GPU memory.

## Alternatives considered

- **Implement `cmd_memcpy` with a compute copy kernel to keep the API
  address-only.** Rejected: a shader copy is slower (shader path vs transfer
  engine), needs a distinct kernel/pipeline, and clashes with the existing
  `Stage::TRANSFER` semantics. The blog itself delegates persistent/large
  copies to the driver's copy command, which in Vulkan is buffer-object
  based.
- **Expose `(buffer, offset)` pairs from `GpuAllocation` and rebuild the
  handle+offset API.** Rejected: that is the retained-mode shape the bindless
  model exists to remove. `&GpuAllocation` keeps the API on the pair
  (`HostPtr`/`GpuPtr`), and the buffer object stays an internal carrier.
- **Add sub-region copy + offset args now.** Deferred: no consumer needs it
  yet, and whole-block copy is the minimal verifiable unit.

## Consequences

- `GpuAllocation` buffers gain transfer capability for every allocation,
  including `Memory::Gpu` (device-local) output, enabling the blog's
  upload→private-heap copy pattern later.
- `dispatch_indirect` makes dispatch arguments indirect (CPU- or GPU-written)
  — one half of what GPU-driven compute needs; GPU-generated draw arguments
  (hazard-flag barrier) remain the indirect multi-draw milestone.
- The command layer now depends on `GpuAllocation::buffer()`; keeping the
  buffer alive is the same ownership as the allocation itself, no new
  lifetime surface.
- clippy/fmt clean, tests pass on lavapipe (Linux CI); MoltenVK/lavapipe
  compatibility unchanged (both support sync2 copy and indirect dispatch).
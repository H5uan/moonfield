# Agent Note: Address-based commands and GPU-address timestamps (VK_KHR_device_address_commands)

Status: implemented

[中文](2026-09-12-device-address-commands-and-gpu-timestamps.zh.md)

## Problem

The command surface took buffer handles plus offsets for indirect
draws/dispatches and buffer copies (`cmd_draw_indirect(buffer, offset, …)`),
while the engine's memory model is address-first — a `GpuAllocation`'s buffer
is a pure address carrier, so the handle/offset pair only restated the
`GpuPtr` the caller already had. The crate also had no GPU profiling path:
classic timestamp queries end in a `vkGetQueryPoolResults` host sync.

## Decision

- `VK_KHR_device_address_commands` is a required device extension (creation
  fails with the name listed when missing, like every required extension);
  its single feature bit is requested and the loader joins
  `DeviceExtensionFunctions`.
- The indirect/copy commands consume device addresses:
  `draw_indirect(args: GpuPtr, draw_count, stride)` (the offset parameter is
  gone — callers use `GpuPtr::offset`),
  `draw_indirect_count(args: GpuPtr, count: GpuPtr, max_draw_count, stride)`,
  `dispatch_indirect(args: GpuPtr)`, and
  `cmd_memcpy(dst: GpuPtr, src: GpuPtr, size)` through `vkCmdCopyMemoryKHR`.
  Every address range is flagged `FULLY_BOUND` — `GpuAllocation` buffers are
  fully bound.
- `Stage::DRAW_INDIRECT` joins the barrier vocabulary, pairing with the
  existing `Access::INDIRECT_COMMAND_READ` for indirect-argument hazards.
- `sync.rs` gains `TimestampQueryPool`: `new(device, count)` plus
  `timestamp_period_ns` (the physical device's `limits.timestampPeriod`,
  cached on `Device` at creation). `CommandBuffer` gains
  `reset_timestamps` (core pool reset before a pass's writes),
  `write_timestamp(queries, index, stage)` (sync2 `vkCmdWriteTimestamp2`,
  single-stage contract), and `resolve_timestamps(queries, first, count,
  dst: GpuPtr)` — `vkCmdCopyQueryPoolResultsToMemoryKHR` writing contiguous
  `u64` tick values (64-bit results with `WAIT`). The CPU reads the mapped
  allocation after the submission's timeline point; no host sync.

## Alternatives considered

- **Manual `get_device_proc_addr` loading.** Only needed when ash lacks the
  extension; the pinned ash master ships the loader, so the aggregated
  `DeviceExtensionFunctions` pattern covers it.
- **Availability pairs (`WITH_AVAILABILITY`) instead of `WAIT`.** Plain
  values with `WAIT` match the reference shape and halve the result
  footprint; the resolve is recorded after the writes in the same
  submission, so availability is ordered.
- **Keeping handle/offset signatures and resolving handles internally.**
  Two command shapes for one memory model; the address forms drop the
  offset parameter and match the pointer model the rest of the crate
  speaks.

## Consequences

- No public command names a buffer handle anymore for the
  indirect/copy/timestamp paths; `GpuAllocation::buffer()` shrinks to the
  upload/readback internals.
- GPU timestamps cost no host synchronization: profiling data lands in
  mapped memory, converted with `timestamp_period_ns`.
- Devices without the extension fail device creation with the name listed;
  every targeted driver (the T1000 included) exposes it.
- `gpu_tests::timestamps` covers write → resolve → host-read end to end;
  `indirect_draw` and `bindless_memcpy_dispatch_indirect` exercise the
  address forms.

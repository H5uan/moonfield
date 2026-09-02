# Agent Note: complete GpuAllocation buffer usage set

Status: implemented

[中文](2026-09-01-gpu-allocation-usage-flags.zh.md)

## Problem

`GpuAllocation` created its address-carrier buffer with only
`SHADER_DEVICE_ADDRESS | TRANSFER_SRC | TRANSFER_DST`. Two consumers violated
the spec, tolerated silently by the NVIDIA driver:

1. `CommandBuffer::dispatch_indirect` passes the buffer to
   `vkCmdDispatchIndirect`, which requires the buffer to be created with
   `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT` (VUID-vkCmdDispatchIndirect-buffer-
   02709).
2. `DescriptorHeap` backing allocations are `GpuAllocation`s, but the
   descriptor-heap proposal mandates that heap backing buffers be allocated
   with `VK_BUFFER_USAGE_DESCRIPTOR_HEAP_BIT_EXT`.

## Decision

The usage set is now
`SHADER_DEVICE_ADDRESS | TRANSFER_SRC | TRANSFER_DST | INDIRECT_BUFFER |
DESCRIPTOR_HEAP_EXT`. The address-carrier design is "one allocation, every
bindless access mode", so the usage set is the superset of those modes; extra
flags cost nothing because memory requirements derive from the buffer object
itself.

## Alternatives considered

- A per-allocation usage parameter so each consumer declares exactly what it
  needs: rejected as needless precision — the carrier's contract is precisely
  that consumers never think about the buffer object.
- Adding `STORAGE_BUFFER` preemptively for future buffer descriptors in the
  resource heap: deferred until a descriptor of that type is actually
  written.

## Consequences

- Both VUID violations are closed; the heap backing and indirect-args paths
  are now spec-clean, not merely driver-tolerated.
- All moonfield-rhi tests pass unchanged on the real driver.

# Agent Note: complete GpuAllocation buffer usage set

Status: implemented

[English](2026-09-01-gpu-allocation-usage-flags.md)

## Problem

`GpuAllocation` 的地址载体 buffer 原先只带
`SHADER_DEVICE_ADDRESS | TRANSFER_SRC | TRANSFER_DST`，两处消费违反规范，
NVIDIA 驱动静默容忍：

1. `CommandBuffer::dispatch_indirect` 把该 buffer 传给 `vkCmdDispatchIndirect`，
   后者要求 buffer 创建时带 `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT`
   （VUID-vkCmdDispatchIndirect-buffer-02709）。
2. `DescriptorHeap` 的堆 backing 就是 `GpuAllocation`，而 descriptor-heap
   提案明确规定堆 backing buffer 必须以
   `VK_BUFFER_USAGE_DESCRIPTOR_HEAP_BIT_EXT` 分配。

## Decision

usage 集合改为
`SHADER_DEVICE_ADDRESS | TRANSFER_SRC | TRANSFER_DST | INDIRECT_BUFFER |
DESCRIPTOR_HEAP_EXT`。地址载体的设计语义本就是"一块分配，承担 bindless
的全部访问方式"，usage 取这些方式的超集；多带 flag 零代价——内存需求本就
从 buffer 对象自身推导。

## Alternatives considered

- 给每次分配传 usage 参数、各消费方精确声明：作为过度精确拒绝——载体的
  契约恰恰是消费方无需感知 buffer 对象。
- 预加 `STORAGE_BUFFER` 供将来 buffer 描述符进资源堆：推迟到真正写入此类
  描述符时再加。

## Consequences

- 两处 VUID 违反闭环：堆 backing 与 indirect args 路径从"驱动容忍"变为
  规范干净。
- 全部 moonfield-rhi 测试在真实驱动上不变通过。

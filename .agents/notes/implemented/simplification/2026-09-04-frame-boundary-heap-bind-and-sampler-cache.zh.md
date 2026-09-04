# Agent Note: Heap binding at the frame boundary and an immortal sampler cache

Status: implemented

[English](2026-09-04-frame-boundary-heap-bind-and-sampler-cache.md)

## Problem

descriptor heap 的绑定是每个消费者自己的事：`record_egui` 在自己的 pass 前绑定，
没有任何机制保证帧命令缓冲一定绑过 heap——core 3D pass 之所以能跑，只是它的
shader 不碰任何 heap slot；一旦往那里加一个采样 shader，就会对着未绑定的 heap
运行。sampler slot 是同样的形状：每个消费者自管一套（egui pipeline 按选项缓存、
drop 时释放；每个 offscreen target 独占一个 slot、resize 时 retire）——同一个
缓存被复制了三份，带三套生命周期故事。

## Decision

- 帧循环在每条帧命令缓冲 `begin` 之后立即绑定两个 heap（heap 绑定以命令缓冲
  为作用域）。直接持有命令缓冲的所有者——测试——自行绑定。
- `DescriptorHeap::sampler_for(desc)` 为每个 `SamplerDesc` 缓存一个 slot 且
  永不释放：不同描述的数量由配置空间天然封顶，一个描述一个 slot 的代价低于
  任何引用计数。`free_sampler_slot` 与 `SamplerSlot` 退役动作一并删除；
  sampler slot 按设计永生。

## Alternatives considered

- **给缓存的 sampler 加引用计数。** 拒绝：那套机制（计数、释放路径、退役
  动作）服务的只是配置空间已经封顶的少数几个 slot。
- **保留各消费者自建的缓存。** 拒绝：同一个缓存复制三份、三套生命周期故事，
  换来的还是那几个 slot。

## Consequences

- 每条帧命令缓冲从 `begin` 起就绑好了 heap；帧内录制的任何 pass 都可以采样
  heap slot。
- egui pipeline 的 sampler map 与它的 `Drop` 消失（`update_texture` 改收
  `&EguiPipeline`）；offscreen target 按描述共享同一个 sampler slot，
  `HeapSlots` 只退役 image slot。
- sampler slot 分配器的 freelist 永远不会被用到；image slot 的 freelist
  仍服务于退役的 image slot。

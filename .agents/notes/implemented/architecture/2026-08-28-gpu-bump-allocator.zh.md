# Agent Note: GpuBumpAllocator —— grow-on-overflow 竞技场分配器，CPU/GPU 双视图共偏移对齐

Status: implemented

[English](2026-08-28-gpu-bump-allocator.md)

## Problem

`GpuAllocation`（bindless）每次分配都创建一个 `VkBuffer`、映射和设备地址——这对持久资源是对的，对上传统来说是错的：每次上传都重建 staging buffer、command pool 并阻塞等队列，正是本阶段要消除的模式。上传路径需要一个廉价的子分配器，从长生命周期的 host-visible 块里切出 (cpu, gpu) 指针对。

## Decision

`moonfield-rhi` 的 `vulkan/bump.rs` 拥有 `GpuBumpAllocator<'a>`，即参考项目（`no_gfx_api`）`Arena` 的形状：

- 块是 `GpuAllocation`（`Memory::Default`，CpuToGpu）；`new` 创建第一块，后续块在溢出时增长——绝不做可能踩到 in-flight 数据的环形回卷。
- 每块记录它建立时的底座对齐；请求对齐超过当前块底座的分配会通过 `GpuAllocation::new_aligned` 增长（或重建）一块——该函数在分配前抬高 `requirements.alignment`，即参考项目的 `mem_requirements.alignment = max(.., align)`。
- `alloc` 按 GPU 基址计算对齐偏移，两条视图共用同一偏移；`check_co_align` 在基址差不是请求对齐的倍数时报错。
- `free_all` 复位到第一块；调用方在帧信号之后调用它，保证竞技场供给的每一次拷贝都已执行完毕。`block_count` 报告已经增长了多少块。
- `BumpAlloc` 携带 CPU 指针、设备地址，以及供 `cmd_copy_buffer` 使用的所属 buffer 和偏移。

`GpuAllocation::new` 保持签名不变（委托给 `new_aligned(.., 16)`），既有的 12 个调用点不受影响。

## Alternatives considered

**博客的环形回卷（溢出时 `offset = 0`）。** 否决：回卷可能覆盖 in-flight 帧仍在读取的区域。竞技场改为增长，复用交给帧信号之后的 `free_all`。

**单块定长。** 否决：溢出会失败而不是增长，迫使按每帧上传峰值浪费地规划大小。

## Consequences

- 上传从长生命周期块中切取——不再有每次调用的 staging buffer、command pool 或队列等待；帧上传器将在其之上构建。
- 超过 16 字节的对齐以一块底座抬高的增长块为代价；16 及以下的对齐使用初始块，无额外开销。
- CPU/GPU 共对齐不变量在建块时校验；驱动或分配器的回归会以错误形式暴露，而不是静默给出错位指针。
- `BumpAlloc::src`/`src_offset` 由帧上传器的拷贝录制消费（下一步）。
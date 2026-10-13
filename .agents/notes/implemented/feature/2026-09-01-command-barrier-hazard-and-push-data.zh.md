# Agent Note: command barrier hazard flags and push data

Status: implemented

[English](2026-09-01-command-barrier-hazard-and-push-data.md)

## Problem

bindless 2.0 的命令面还缺博客愿景的两块。阶段级 barrier（`CommandBuffer::barrier(Stage, Stage)`）始终只编排普通内存读/写 hazard：无法表达涉及描述符堆的 hazard（CPU 通过 host 映射写描述符、着色器经非均匀堆索引读取）。而扩展自带的 push-constant 替代品 `vkCmdPushDataEXT`（扩展规范："update the values of push data"，经既有 PushConstant 存储类对所有着色器可用，是 shader-constant 数据设备地址的 fast path）没有任何 RHI 封装。

## Decision

- `BarrierHazard` 枚举（`bindless.rs`，`Stage` 旁）：`Memory`（原行为——两侧 MEMORY_READ|MEMORY_WRITE）与 `Descriptors`，其目标 access 额外暴露 `SHADER_SAMPLED_READ`（着色器经堆描述符执行的采样图像读取）。`barrier(before, after, hazard)` 取代双参数形式；现有两处调用点传 `BarrierHazard::Memory`。
- `CommandBuffer::push_data(offset, data)`：以调用方字节上的 `HostAddressRangeConstEXT` 封装 `vkCmdPushDataEXT`。与 push constant 一样按偏移寻址、对所有着色器阶段可用，录制时受 `max_push_data_size` 约束（越界由 validation 报错；RHI 不预检——还没有消费者需要该限制）。

## Alternatives considered

- 把 `max_push_data_size` 带上 RHI 类型并在 `push_data` 里拒绝超大写入：目前没有消费 root data 的 push-data 管线，YAGNI。
- 把 hazard 拆成 bitflags 集合：目前只有两种，默认枚举让调用点保持可读，等更多种类到来再改。

## Consequences

- 描述符堆写入（CPU 或先前 GPU 阶段）现在可以显式地相对采样排序，而不是只能强制宽泛内存 access mask。
- `push_data` 给管线提供比 push constant 更大、按偏移寻址的 root-data 通道，为 phase-4 管线集成就绪。
- 测试：`bindless_barrier` 现在两种 hazard 都经 dispatch 对运行（memory + descriptors）；`command_push_data` 验证不重叠偏移可干净录制。

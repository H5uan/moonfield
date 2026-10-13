# Agent Note: RHI sync and memory contract narrowing

Status: implemented

[English](2026-10-11-rhi-sync-memory-contract-narrowing.md)

## Problem

[RHI 分层提案](../../implemented/architecture/2026-10-10-rhi-interface-and-backend-layering.md)要求在适配两个后端之前先定义共同契约，并警告按 Vulkan 形状定义的契约可能迫使 Metal 做昂贵或错误的模拟。有三块契约位于这条断裂带上：

- **屏障。** 公开的 `Stage`/`Access` 词汇是 Vulkan 形状。在 Metal 上，`TRANSFER` 和 `DRAW_INDIRECT` stage 在 `MTLStages` 映射中被合并或丢弃，`barrier` 的 access 参数被完全忽略。依赖 access 粒度的调用方在 Metal 上既得不到保证，也收不到错误。
- **同步原语。** 公开面带有 `Fence`、二元 `Semaphore` 和 `TimestampQueryPool`。在 Metal 上信号量只是空壳，等待发生在提交内部；fence 和时间戳类型根本不存在。
- **命令池与内存。** `CommandPool` 暴露了逐后端不同的 queue-family 选择和失败模式；`Memory` 在 Vulkan 上是分配类别，在 Metal 上却持有一个 buffer。

让 Metal 去满足这些 Vulkan 形状的契约，等于模拟该 API 并不具备的语义。落地的方向是把公开契约收窄到两个后端都能真实兑现的语义范围。

## Decision

公开的同步、命令和内存契约只命名两个后端原生支持的操作。

- **内存驻留类别与分配对象分离。** `MemoryResidency`(`GpuOnly`、`CpuWriteOnly`、`CpuReadOnly`）是 `core/src/types.rs` 的 core 词汇，host 可见性与 GPU 完成保证逐类别定义。`GpuAllocation` 在两个后端上是同一构造器形状（`new`/`new_aligned`，返回 `Result`）与一致的访问器（`gpu_ptr`、`host`、`read_bytes`、`size`)。Metal 那个持有 buffer 的 `Memory` 类型被消解：`GpuAllocation` 持有自己的 buffer,bump 分配器和 root ring 使用 crate 内部的 `Buffer`。Metal 把类别映射到 storage mode(host 可见类别用 shared,`GpuOnly` 用 private);Vulkan 映射到 allocator location，语义与旧 `Memory` 一致。
- **stage 粒度的屏障，公开面不再有 access 掩码。** `Stage`(`VERTEX`/`FRAGMENT`/`COMPUTE`/`TRANSFER`/`ALL_GRAPHICS`/`ALL`）是 core 词汇；`Access` 已删除。`CommandBuffer::barrier(before, after)` 是唯一的屏障入口。每个后端自行推导原生 stage/access 对：`stage_to_vk` 保守映射（着色器 stage 蕴含读/写/采样，`ALL*` 蕴含 memory read/write),`stage_to_metal` 把 Metal 4 无法表达的类别放宽到其完整 stage 集——绝不收窄、绝不静默丢弃。`DRAW_INDIRECT` 不在词汇中（无消费者）。提案中的纹理 `transition` API 未落地：目前没有 pass 需要显式 layout 控制，它将随第一个消费者到来，而不是提前建造。
- **只有 timeline 的同步。** 设备持有一条提交时间线（`Timeline`，带 `wait`/`completed`,Vulkan 上是 timeline semaphore,Metal 上是 shared event)。`Device::submit_frame(cmd, acquired)` 等待每个已获取图像的槽位同步、信号其 present 同步、以帧号信号时间线并返回帧号。`Swapchain::acquire` 返回 `AcquiredImage`；未经 present 就 Drop 即 abort（槽位的 acquire 信号量在下一次 acquire 时惰性重建，`take_aborted` 馈给 surface 的 recreate 标志）。二元信号量位于 swapchain 槽位内部；`Fence` 被删除（全库无消费者）;`TimestampQueryPool` 降为 test-only，直到出现 capability 形态的消费者；Metal 的空壳 `Semaphore` 被删除。
- **由队列发放池化的命令缓冲。** `CommandPool` 已私有化。`Device::command_buffer(usage)` 从设备自有的池返回一个已开始录制的缓冲；`begin`/`end`/`reset` 是 crate 内部方法，提交入口对缓冲做幂等 end。窗口帧循环每帧取新缓冲，不再维护 per-slot 环；缓冲在 drop 时自行归还池中。
- **shader target 与 capability 从后端选择派生。** facade 的 `shader_target()` 与 `shader_capabilities()` 是按后端 feature 固定的 `const fn`；消费者不再写出 `Spirv` 或 `spvDescriptorHeapEXT`。两个后端的设备都提供 `shader_cache()`;`Device::new` 签名统一（surface 参数在 Metal 上为惰性）。

## Alternatives considered

- **保留公开 access 掩码，Metal 对无法兑现的项报错。** 否决：每个 access 常量在其中一个后端上都会变成 no-op 或错误，契约承诺了实现无法交付的效果——正是分层提案警告的模拟陷阱。最高频的调用点（GS compute 链）需要的是 dispatch 之间的写后读，stage 对已经能表达。
- **自动的逐资源状态跟踪。** 否决：bindless 裸 GPU 指针使读写跟踪无法在不重设计资源模型的前提下表达，[render pass schedule redesign](../../implemented/architecture/2026-09-09-render-pass-schedule-redesign.md) 已记录这一点。显式 stage 对让同步在调用点保持可见。
- **`CommandPool` 保持公开、只统一签名。** 否决：统一后反正要隐藏 queue-family 选择，而唯一的消费模式只是"每帧拿一个可录制的缓冲"。池是簿记而非契约。
- **`Fence` 和二元 `Semaphore` 保持公开。** 否决：swapchain 和帧步调内部之外没有消费者。`Fence` 在全库无任何消费者，已删除；`Semaphore` 降为 crate 内部；未来确有需求时，可以重新引入一个在两个后端都有真实语义的类型。
- **公开的放置式堆（placement heap）与预算查询。** 暂缓：消费者目前只需要 device-local、上传、回读三类。等出现消费者时再加放置式 API。

## Consequences

- facade 两个后端的 re-export 列表在全部同步、内存、命令类型上已完全一致；剩余分歧集中在 capability 形态的区域（bindless descriptor heap、离屏目标、时间戳查询、上传），归分层提案的 capability 工作所有。
- `RenderContext` 的门切换状态机只记录 stage 对；窗口帧循环以设备时间线自定步调，其 sequencer 每次提交都与设备帧计数对断言，计数脱节会 panic 而不是让时间线静默失步。
- 重写帧 abort 路径时发现旧 `abort_frame` 从未满足过自己的不变量（abort 后上下文并不可 begin);`FrameSequencer::note_abort` 现在定义了该语义，之前失败的窗口测试已通过。
- 契约测试套件（`crates/moonfield-rhi/tests/contract_memory.rs`）经 facade 公开 API 固定分配/回读/驻留契约，对构建选中的任后端运行；屏障顺序与令牌等待由两后端的 gpu_tests 固定。首次 macOS 运行（Apple Silicon）验证了 Metal 半边：完整套件通过，包括 compute 契约的 pending-barrier encoder 重放。该运行抓到 core 编译器为 Metal 目标记录源级入口名、而 Slang 的 Metal 代码生成会把 `main` 改名为 `main_0`（警告 E40100）的问题；`finish_compile` 现已应用该改名，`main` 命名的源在两目标上都能编译。硬件跳过仍不等价于通过 —— 携带所需扩展的 Vulkan 设备仍须运行该套件。
- 纯 stage 屏障与此前的全局屏障同样保守，过度同步没有变得更糟；若性能分析显示有代价，应扩大 stage 集合，而不是重新引入 access 掩码。

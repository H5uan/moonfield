# Agent Note: Render recording context

Status: implemented

[English](2026-09-09-render-recording-context.md)

## Problem

[重设计 note](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md) 规定了封闭的录制面——带三扇类型化门的 `RenderContext` 和 barrier 阶段状态机——但 M0–M5 只落了 schedule 骨架。每个录制系统都通过 `FrameContext::current_command_buffer()` 裸取帧 command buffer：opaque pass、孤儿目标清理、egui pass、splat sort 全部绕过了预定的门，compute 门根本不存在。`TrackedRenderPass` 只是 `record_view_pass` 内部的局部包装，仅对管线 bind 去重，且用管线的 Rust 地址当键——只靠"管线在 `PrepareViews` 重建"这条约定保平安，对 viewport、depth、cull 状态全盲。跨 pass 同步只靠 schedule 顺序加各 pass 硬编码的 layout 值；rhi 的 `CommandBuffer::barrier` 在渲染层没有调用者。

## Decision

- `render-core` 新增 `context` 模块。`RenderContext` 是 `SystemParam`（exclusive 系统内也可经 `RenderContext::get(world)` 直接构造），持有帧的 `Ref<FrameContext>` 和 `RefMut<RecordingState>`；资源缺失时两扇门都读作"无"，pass 在 headless 机器和帧外自然 no-op。三扇门：`begin_rendering(&RenderPassDesc) -> TrackedRenderPass`、`compute() -> ComputeRecording`、`barrier(before, after, hazard)`。
- `RecordingState` 是 render-world 资源，承载状态机相位（`Idle → Rendering/Compute`）。`acquire_window_frames` 在帧开始时插入全新实例；`submit_window_frames` 丢弃它。门切换自动插入 rhi 的全局、无资源的内存 barrier：compute 相位间用 `COMPUTE → COMPUTE`；凡涉及光栅输出（或可能涉及）的切换用最宽的 `ALL → ALL`——`Stage` 没有 `COLOR_ATTACHMENT_OUTPUT` 常量，保守组合是唯一可表达的。手动 `barrier` 按给定记录并把相位复位为 `Idle`，标记 hazard 已处理。
- `TrackedRenderPass` 移入 `context` 模块，是 `begin_rendering` 的返回值。bind 跟踪经新增的 `GraphicsPipeline::id()` 用管线的原始 Vulkan 句柄作键（活管线间唯一），并新增 viewport、depth-state、cull-state 的去重；跟踪在 `begin_rendering` 重置。scissor 与 blend 是不去重的透传（UI pass 逐 draw 修改）。
- `ComputeRecording` 包装 compute 门：`bind_pipeline`、根数据 push、`dispatch`，首次之后的每次 dispatch 前自动插 `COMPUTE → COMPUTE` 内存 barrier——这是每个 compute pass 都有的读后写链。`RadixSort::record` 改收它，手写的三个逐 pass barrier 删除；状态机发出的是同一组。
- `record_view_pass` 经光栅门录制，函数体拆出 `record_view_items`（动态状态、view uniforms、item 分发），自持 command buffer 的测试走同一路径。egui pass、孤儿目标清理、splat sort 全部走门；`FrameContext::current_command_buffer` 收为 crate 私有。

## Alternatives considered

- **`RenderContext` 持有 `CommandBuffer` 克隆。** 否决：`CommandBuffer::drop` 会把 Vulkan command buffer 归还给池，克隆即双重释放；`Ref`/`RefMut` 对借用既有 per-slot buffer，零 unsafe。
- **光栅切换用最小 stage 组合**（`FRAGMENT → COMPUTE` 等）。否决：光栅写发生在 attachment output，`Stage` 无法命名；少命名会在 composite 顺序上留下真 hazard，最宽组合才是诚实的选择。若 `Stage` 未来增加 attachment-output 常量再回看。
- **只在触及相同 buffer 的 compute dispatch 间自动 barrier。** 否决：rhi 的 barrier 刻意无资源（bindless 指针），逐资源跟踪不可表达；保守全局 barrier 就是[重设计 note 规定的形式](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md)。

## Consequences

- 渲染层不再有系统点名 `CommandBuffer`；rhi 裸面只能经三扇门或自持 command buffer（测试）触达。
- 每次光栅→光栅、compute→光栅切换现在都带全局 barrier——对 clear 是过同步，对读后写链是构造即正确（GS composite 落在这条路上）。
- pass 系统里的 `begin_frame_draw_arena`、`FrameContext` 检查、逐 pass 的 `TrackedRenderPass::new` 样板消失；新 pass 打开一扇门即可录制。

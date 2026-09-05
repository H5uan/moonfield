# Agent Note: single frames-in-flight constant owned by the RHI

Status: implemented

[English](2026-09-06-frames-in-flight-single-source.md)

## Problem

退役环深度(`moonfield_rhi::RETIRE_RING`)与帧循环的在飞帧数
(`moonfield_render_core::MAX_FRAMES_IN_FLIGHT`)必须相等,否则某个槽位可能在其
提交完成前就被排空。此前两者是两个独立的 `usize = 2` 常量,只靠
`RenderPlugin::build` 里的运行时 `assert_eq!` 保持一致——一个编译期事实被放到运行
时检查;而第三个消费者(编辑器的 `EguiFrameResources`)则把该数量作为参数,信任
调用方传对。

## Decision

`moonfield_rhi::RETIRE_RING` 是唯一权威来源。`moonfield_render_core::
MAX_FRAMES_IN_FLIGHT` 改为它的常量别名,下游名称(`render-feature` 的
`FrameDrawArena`、编辑器的 `EguiFrameResources::new(device,
MAX_FRAMES_IN_FLIGHT)`)保持不变。`RenderPlugin::build` 中的 `assert_eq!` 删除——
相等性现在由定义保证。该值由 RHI 持有,因为退役环深度是帧循环必须遵守的约束,而
RHI 不能依赖 render-core。

## Alternatives considered

- **由 render-core 持有该值,RHI 通过 `Device` 参数接收深度。** 否决:为一个帧循环
  的固定策略做构造函数管道,且每个直接构建 `Device` 的测试或工具都得知道正确的数
  字。
- **建一个共享常量 crate。** 否决:不值得为一个常量建一个 crate。

## Consequences

- 修改深度只需在 `moonfield-rhi` 改一处;失配这一类错误在编译期被消除。
- `RETIRE_RING` 的文档注释现在写明别名方向,后来的读者能知道哪个名称是权威。

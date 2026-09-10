# Agent Note: Render pass surface cleanup

Status: implemented

[English](2026-09-09-render-pass-surface-cleanup.md)

## Problem

pass 系统评审留下的四项尾巴：`ViewQuery` 在[重设计 note](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md) 的 M2 落地时就没有消费者（per-view pass 是 exclusive 系统、手工读 `CurrentView`，于是两套并行的 per-view 访问机制并存）；`FrameDrawArena`——帧分页 GPU scratch，与 `FrameContext` 同级的引擎基础设施——住在 render-feature，且带着为 mesh 管线结构体硬编码的两个分配器（`alloc_view_uniforms`、`alloc_draw_data`）；`MOONFIELD_DEBUG_SCENE` 日志接缝嵌在 `record_view_pass` 的录制体内；splat sort pass 用自由函数 `register_sort_pass(app)` 注册，与全库其他部分的 plugin 模式不一致。

## Decision

- 删除 `ViewQuery`。`CurrentView` 加 exclusive 系统读取是唯一的 per-view 访问机制；将来若出现非 exclusive 的 per-view 系统再带回参数形态。
- `FrameDrawArena` 下沉到 render-core（`arena` 模块），连同 `begin_frame_draw_arena` 一起由 `RenderPlugin` 注册进 `PrepareViews`。两个具名分配器合并为一个泛型 `alloc<T>()`；`ViewUniforms` 与 `DrawData` 留在 render-feature——与 shader 的布局契约才属于那一层。
- 调试接缝改为 `debug_scene_log`，一个排在 `queue_opaque_3d` 之后的 `Queue` 集系统：场景内容在入队时打日志，pass 体内只剩 GPU 工作。同样的环境变量、同样的每进程一次语义，按 view 输出。
- `register_sort_pass` 变为 `SplatSortPassPlugin`。它保持可选（验收测试自行添加；GS 路线图 M3 会接入真实链路），但注册面从此与其他一切一样是 plugin。

## Alternatives considered

- **把 `ViewQuery` 扩展成元组来证明其存在。** 否决：没有消费者的组合能力正是这次评审在清除的表面增长；而且 exclusive 的 pass 系统本来就用不了 `SystemParam`。
- **保留 `alloc_view_uniforms`/`alloc_draw_data` 具名包装。** 否决：这两个名字编码的是两个调用点而非契约，每个未来 pass 都会往引擎基础设施上加一个方法；`alloc<T>()` 是同一机制，把类型放回调用点。
- **现在就由 `RenderFeaturePlugin` 注册 `SplatSortPassPlugin`。** 延后：splat 提取落地前 sort 的 pair 是合成的，给每帧编辑器接上无意义的 GPU dispatch 毫无收益。

## Consequences

- per-view 访问一个机制、帧 scratch 分配一个机制、调试日志住在数据所在处（queue）、feature 注册全部 plugin 化。
- render-feature 不再拥有引擎级 GPU 基础设施；其 `render_phase.rs` 缩减为 phase item、draw command 与入队逻辑。

## Verification

- `cargo fmt`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`python3 scripts/verify_agents.py` 全部通过。

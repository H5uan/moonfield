# Agent Note: Render set chain and per-view schedule

Status: implemented

[English](2026-09-09-render-set-chain-and-per-view-schedule.md)

## Problem

`Render` schedule 靠函数名约束排序：`main_opaque_pass_3d` 是一个单体系统——惰性建管线、自己遍历 view、每个 target 只渲染第一个 primary view；排序混在 queue 系统里；`Core3dFrame` 资源每帧重建以携带 per-view phase；editor 在编译期锚定 render-feature 的系统名。加一个 pass 就要改这个单体。

## Decision

- moonfield-ecs 增加 `SystemSet`：集合是类型化的 no-op 锚系统；`add_sets((A, B, ...))` 按序注册锚链，`in_set::<S>()` 把系统挂在其集合锚之后、下一锚之前（相邻集合因此天然有序，无需显式约束），`before_set` / `after_set` 是单边形式。
- `Render` schedule 是一条集合链——`PrepareAssets` → `Queue` → `PhaseSort` → `PrepareViews` → `CameraDriver` → `PostViews` → `Submit`——由 `RenderPlugin` 注册；`acquire_window_frames` 在链前，`submit_window_frames` 在链后。`RenderPrepare` 与 `RenderQueue` 标签删除。
- phase 是 view 实体组件：每个抽取 view 上的 `RenderPhase<Opaque3d>`，由 `prepare_view_phases`（`Queue`）挂上、`queue_opaque_3d` 填充（不再排序）、泛型 `sort_phase::<P>` 系统（`PhaseSort`）排序。`Core3dFrame` 与 `Core3dView` 消亡。
- `camera_driver<L>`（render-core）按 `(Camera::order, entity)` 排序 view——`Camera` 增加 `order` 字段——并为每个 view 运行视图 schedule 一次，期间插入 `CurrentView`。当前唯一实例化是 `Core3d`，其锚集合为 `Core3dOpaquePass`；`opaque_pass_3d` 是其中的 per-view 系统。管线（重）建、view-target 附件、draw arena 的每帧 begin 移入 `PrepareViews` 系统。
- 每个 view 都渲染——首 primary view 特例消亡。没有 view 认领的离屏 target 由 `clear_orphan_view_targets`（driver 之后、`PostViews` 之前）清为背景色。
- editor 锚定 `PostViews` 与 `Submit` 集合，不再引用 render-feature 系统名。

## Alternatives considered

- **显式双边挂载（`before_set(X).after_set(Y)`）。** 落选：漏写一边就静默乱序；`in_set` 从链推导两条边，成员资格是一个决定。
- **Bevy 式集合图节点。** 落选：schedule 就是对带名系统的拓扑排序；类型化锚*就是*节点，链编码边——不引入新的执行语义。
- **保留 `Core3dFrame` 作 phase 载体。** 落选：pass 仍然是对每帧重建资源的中心化遍历，而非 view 数据上的 per-view 系统——正是 [redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md) 要移除的形状。
- **经 `ViewQuery` 的参数化（非独占）per-view pass。** 落选：`DrawFunction::draw` 仍吃 `&World`——现在参数化 pass 等于提前重造 draw 路径，那是 M3 的 `RenderCommand` 工作。`ViewQuery` 为第一个参数化客户落地待用。

## Consequences

- 多相机场景按 `(order, entity)` 顺序把每个 view 渲进其 target；同 target 上后一个 view 的 clear 覆盖前一个，即相机叠加语义。
- `ViewQuery` 与 `CurrentView` 是 render-core 公开表面；opaque pass 是独占系统（draw 函数读 world），直到 M3。
- editor 排序不再引用 render-feature 内部；splat 落地时其链锚在 `Core3dOpaquePass` 上。

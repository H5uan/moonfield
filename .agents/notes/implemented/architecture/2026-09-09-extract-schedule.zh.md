# Agent Note: Extract schedule

Status: implemented

[English](2026-09-09-extract-schedule.md)

## Problem

抽取以 `App` 上按注册顺序的闭包列表运行（`FnMut(&World, &mut World)`）：没有排序约束、没有系统参数，逐实体的可选查找只能手写循环。[render pass schedule redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md) 的 M1 用 schedule 取代它，M2 的 set 链建在其上。

## Decision

- `App::render` 在新的 `ExtractSchedule` 运行期间停靠主世界：`mem::take` 取出世界、`World::park_main_world`、清空渲染世界实体（资源保留）、运行 schedule、`unpark_main_world`、取回世界，然后运行 `RenderPrepare` / `RenderQueue` / `Render`。
- `MainWorld`（moonfield-ecs）是穿上 `Send + Sync` 外衣的裸指针。World 两者皆非（无约束的 command 闭包、带借用标志的列），`Resource` 的 blanket impl（`Send + Sync`）把它排除在外，coherence 又禁止手动 impl——因此停靠的世界以指针形式旅行，其有效性是一份停靠契约：只有 `Extract` 解引用它，且只在 schedule 运行期间。
- `Extract<T>`（render-core）是 Bevy 形状的系统参数：`T::fetch` 在停靠的主世界上运行；item 持有资源单元的共享借用，停靠的世界无法在运行中被替换。
- extract 系统经 `Commands` 写渲染世界，每个系统之后即应用，后继 extract 系统能看到先行的 spawn——与闭包时代相同的顺序。闭包列表与 `App::add_extract_system` 删除；所有 extractor 迁移完毕（相机、窗口、`extract_with_transform`、mesh 与 shader 资产、editor frame）。

## Alternatives considered

- **暴露主世界 `&World` 的裸 `Extract` 参数。** 落选：query 引擎当时表达不了抽取所需的形状；改为落地 [composable WorldQuery](2026-09-09-composable-world-query.zh.md)，抽取保持类型导向。
- **用 `Arc<Mutex<World>>` 停靠。** 落选：满足 `Resource` 约束需要全部 command 调用点改用 `Send` 闭包，还要在单线程缝上加锁仪式。
- **保留闭包、schedule 与 M2 一起做。** 落选：set 链消费 `ExtractSchedule`；推迟它只会把里程碑串行化，没有收益。

## Consequences

- extract 系统与其他系统一样用系统参数和排序约束组合；注册方式是 `add_render_systems(ExtractSchedule, ...)`。
- 渲染世界每帧重建实体的现状保留（entity sync 仍在范围外——redesign 的红线）。
- moonfield-app 保持 `#![forbid(unsafe_code)]`：指针工作在 moonfield-ecs（`MainWorld`）与 render-core（`Extract::fetch`）里。

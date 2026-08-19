# Agent Note: Archetype-based ECS storage

Status: implemented

[English](2026-08-19-ecs-archetype-storage.md)

## Problem

早期 `World` 按实体存储组件(`EntityId` 为键),查询时要扫描全部实体并按组件类型运行时过滤。Spawn 与组件插入/移除在热路径上是 O(entities) 或需要昂贵的簿记;组件数据在内存中零散分布,对引擎与渲染器密集循环的缓存不友好;也没有自然的方式按组件集合组织实体。在 [#24](https://github.com/H5uan/moonfield/pull/24) 跟踪的重构,就是为了用 archetype 存储取代这套实现。

## Decision

`moonfield-ecs::world2::World2` 以 **archetype** 存储实体:实体的组件集合决定它所在的 archetype,每个 archetype 用扁平紧凑的切片存放组件(`archetype.rs`)。配套部件:

- `bundle_to_archetype` 缓存 `TypeId` bundle → archetype 查找;`insert_edges` / `remove_edges` 缓存 archetype 间的迁移边。
- `entities` 表保存 `EntityId` → (archetype 索引, 行索引);0 号 archetype 保留给无组件实体,由 `World2::flush` 批量分配。
- `spawn_at` 复用指定的 `Entity`(winit 后端以此收养预建窗口实体)。
- 变更检测基于 **tick**:world 持有 `change_tick` / `last_change_tick`,`increment_change_tick` 每个 schedule run 推进一次;时钟从 1 开始,因此系统初始 `last_run = 0` 会把每个组件视为新变更。
- `query` / `query_mut` 通过 `Query` trait 在匹配的 archetype 上迭代,借用检查由 `borrow.rs` 负责。

迁移期间旧 `World` 仍存在;两个实现将在迁移完成后统一为一个名字。

## Alternatives considered

- **保留实体键 map。** 拒绝:组件过滤与插入/移除的成本随实体总数增长(而非随匹配子集),零散的组件切片也不利于本引擎目标中的紧循环。
- **整体引入 bevy_ecs。** 拒绝:本 workspace 在 `moonfield-ecs` 中自建 ECS 学习线;拉入完整依赖等于交出本 crate 想拥有的设计决策(例如简化的 tick 窗口)。
- **用 sparse-set 存储(实体侧 SOA)而非 archetype。** 拒绝:它在单组件插入/移除上有优势,但在密集 archetype 迭代上吃亏——而后者是这里的支配性模式。

## Consequences

- 查询只迭代匹配的 archetype 并触及连续内存;spawn / despawn 通过 `flush` 批量分配,这是实体 id 获得行下标的唯一节点——需精确理解。
- `World2` 是迁移期的临时名;与旧 `World` 统一命名是后续工作。
- tick 窗口 `(last_change_tick, change_tick)` 恰好覆盖自上次 schedule run 以来的写入——不要在 run 中途推进时钟。
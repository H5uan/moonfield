# Agent Note: Composable WorldQuery

Status: implemented

[English](2026-09-09-composable-world-query.md)

## Problem

query 引擎为每种形状手写一个迭代器——`&T`、`&mut T`、三种双组件配对、单独的 `Option<&T>`——没有超过二元的元组，也没有元组内的 `Option`。extract 系统需要 `(&Camera, &GlobalTransform, Option<&CameraTarget>, Option<&PrimaryCamera>)`；缺的形状正是 `extract_cameras` 只能手写逐实体 `get_component` 循环的原因。

## Decision

- `WorldQuery` 是 Bevy 式的组合契约：每个元素决定 archetype 归属（`matches`）、为迭代器的生命周期借住自己的列（`borrow_fetch` / `release`，即 archetype 借用标志）、逐行产出 item（`fetch`）。`READ_ONLY` 常量说明查询是否含 `&mut T`。
- 一个通用 `QueryIter` 取代全部逐形状迭代器。`World::query` / `query_mut` / `query_filtered(_mut)` 与 `Query` 系统参数是它上面的薄入口；共享入口拒绝可变查询（独占入口是 `query_mut` 和 `iter_mut`），与原先的 panic 行为一致。
- 元组按合取组合三步协议（宏展开，元数 0–8，与系统参数元组对齐）；`Option<Q>` 匹配所有 archetype，在 `Q` 的列缺失处产出 `None` 行。
- 逐实体 `Query::get` 仍只支持单组件。

## Alternatives considered

- **扩充逐形状迭代器集合。** 落选：每种新组合都要再手写一个迭代器；组合契约消除的正是这个组合爆炸。
- **Bevy 的 `State` / `Fetch` / `set_archetype` 机制（unsafe-cell world 之上）。** 落选：moonfield 的安全模型是 archetype 列上的运行时借用标志；三步协议在同一模型下 delivers 同等组合能力，无需引入 unsafe-cell 底座。
- **用一个暴露 `&World` 的 `Extract` 参数绕过需求。** 落选：它绕开而非补齐缺失的形状，并且让每个未来的 extractor 放弃类型导向的查询声明。

## Consequences

- 查询调用点不变（`world.query::<Q>()` 写法照旧）；公开表面上的 `Q::Iter` 消失——`QueryIter<'w, Q>` 是唯一迭代器。
- 全部既有查询测试在新引擎上原样通过；五个新测试钉死元组合取、共享/可变元组中的 `Option`、单独 `Option` 的等价性、共享入口对可变访问的拒绝。
- [extract schedule](2026-09-09-extract-schedule.zh.md) 是新形状的第一个消费者。

# Agent Note: 原型级查询过滤器（With/Without/Or）

Status: implemented

[English](2026-08-19-query-filters.md)

## Problem

此前的查询只能表达"恰好拥有这些组件的实体"——即取数本身。像"有
Transform 但不是子节点"或"有 A 或 B"这类常见选择，要么把并不读取的组
件也一并取出（白白支付借用与带宽），要么在循环体内逐实体手写成员检
查，每帧都付一次。

## Decision

`moonfield-ecs` 新增 `filter.rs`：`QueryFilter` trait，包含 `With<T>`、
`Without<T>`、`Or<(…)>`（析取）、至多八元组（合取）以及 `()`（不过
滤），命名沿用参考实现的查询过滤器。

- 所有过滤器都是**原型级（archetypal）**的：
  `QueryFilter::matches_component_set` 在构造迭代器时，对每个 archetype
  的组件类型集合只求值一次，从不接触逐实体数据。过滤的代价是每个
  archetype 一次探测——这正是 archetype 存储的天然形态，也是过滤器表
  面刻意保持这么小的原因。
- `Query` 系统参数新增带默认值的第二类型参数：`Query<Q, F = ()>`。
  `iter`/`iter_mut`/`get` 都遵守 `F`（`get` 在取数前就会拒绝不在匹配
  archetype 中的实体）。
- `WorldQuery` trait 的 `fetch`/`fetch_mut_cell` 变为默认方法，委托给新
  增的 `fetch_with`/`fetch_mut_cell_with`（接受一个 archetype 谓词）；
  每个迭代器构造函数都通过该谓词计算命中列表。`Option<&T>` 的迭代器此
  前遍历所有 archetype（没有命中列表）；现在在构造时记录通过过滤的
  archetype 集合。
- 命令式入口同步提供：`World::query_filtered::<Q, F>()` 与
  `query_filtered_mut::<Q, F>()`。
- trait 的探测参数是 `&dyn Fn(TypeId) -> bool` 闭包而不是 archetype 本
  身，因此 crate 私有的 `Archetype` 类型不会出现在公开签名里。

## Alternatives considered

- **逐实体过滤器。** 否决：`With`/`Without`/`Or` 在 archetype 存储上不
  需要逐实体求值；增加这条轴只会招来意外变慢的过滤器。变更检测类过滤
  器（`Added`、`Changed`）才需要它，本次未移植。
- **在 `WorldQuery` 内部做类型级过滤组合。** 否决：让查询项本身携带过
  滤器（例如 `(Q, F)` 查询对）会把取数逻辑与选择逻辑缠在一起；把 `F`
  放在 `Query` 参数上与参考实现的形状一致，也让 `WorldQuery` 在精神上
  保持不变。
- **在过滤器 trait 中暴露 `Archetype`。** 否决：该模块是 crate 私有
  的；`TypeId` 探测闭包以每个 archetype 一次间接调用的代价保住了边
  界。

## Consequences

- 单参数的 `Query<&T>` 不受影响——`F` 默认为 `()`。
- 过滤与未过滤的迭代器共用构造函数；只有一条代码路径，不过滤的情形编
  译后与之前的扫描完全相同。
- `Added`/`Changed` 过滤器、以及单组件 `get` 形状之外的逐实体取数过滤
  未移植；它们需要按实体查 change tick，将来在这条接缝上很容易补。

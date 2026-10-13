# Agent Note: Per-system query archetype cache and wired edge tables

Status: implemented

[English](2026-09-19-query-archetype-cache-and-edge-tables.md)

## Problem

`moonfield-ecs` 里有两项开销随世界的历史增长，而不是随存活数据增长：

1. 每次 `Query::iter()`/`iter_mut()` 都重新扫描整个 archetype 列表，并新分配一个
   存放命中项的 `Vec`。archetype 列表单调增长，空 archetype 也从不回收，因此即使
   进入稳态，扫描开销也会随应用运行时间不断增长。
2. `World` 带着移植来的 `insert_edges`/`remove_edges` 表（以及 `InsertTarget`
   值类型）却从不读取：每次 `insert_component`/`remove_component` 都从零重建目标
   archetype 的类型集合（一个 `Vec<ComponentMeta>` 加一个 `Box<[TypeId]>`、一次排序、
   一次整切片哈希查找），`insert_bundle` 还有一次 O(n×m) 的 `contains` 合并，而
   `spawn`/`spawn_at`/`despawn` 每次调用都分配一个只喂给生命周期 hook 循环的
   `Vec<TypeId>`。crate 级的 `#![allow(dead_code)]` 把这些连同更多死代码一起掩盖了
   （`SpawnColumnBatchIter` 及其 `Entities::alloc_many` 支撑代码、
   `Archetype::merge`/`move_to`，以及一个从未被读取的 `AtomicU64` world id）。

## Decision

- `Query<Q, F>` 的 `SystemParam::State` 是新的 `QueryState`
  （system.rs）：命中的 archetype 索引、world id，以及构建缓存时的 archetype
  数量，与 state 原本携带的逐系统变更检测窗口并列（`refresh_window` 在每次 fetch
  前只重写窗口；下面的缓存检查在 `fetch` 内进行）。当 world id 不同或数量变化
  时，`fetch` 重建命中列表——archetype 列表只增不减，数量一致就意味着缓存是完整
  的。迭代基于缓存的索引构建 fetch 列表（query.rs 的 `QueryIter::new_cached` /
  `new_shared_cached`），只有一次精确容量的分配，没有扫描。`Query` 增加了第二个
  生命周期参数（`Query<'w, 's, Q, F>`）来借用该 state，如同 `MessageReader` 借用
  它的 `MessageCursor`。
- 原本闲置的 `World.id` 字段成为缓存的 world 失配防线：一个进程级计数器为每个
  `World` 分配唯一 id（从 1 开始；0 表示尚未构建的 `QueryState`）。这覆盖了同一个
  `SystemState` 对不同 world 取参的情形（例如测试，或 `Extract` 读取停靠的主
  world），这是单靠数量检查无法区分的。
- 命令式的 `World::query`/`query_mut`/`query_filtered`/`query_filtered_mut`
  入口保持扫描：它们是一次性调用，没有可挂载缓存的持久 state，扫描是它们明示的
  代价。
- edge 表以 `(archetype id, TypeId) → 目标 archetype` 的形式接入，记忆化类型集合
  的并/差。`insert_component` 和 `remove_component` 查询
  `insert_edges`/`remove_edges`；`insert_bundle` 查询独立的 `bundle_edges`，以
  bundle 的静态类型 id 为键。没有静态键的 bundle（`DynamicBundle::key() == None`）
  总是重新计算。由于 archetype 只增不减，edge 永不失效。
- `insert_bundle` 对旧 metas 与 bundle metas 的并集改为两个有序列表的归并
  （两者都按 `ComponentMeta::cmp` 排序），取代平方级的 `contains` 循环。
- `spawn_inner`/`spawn_at`/`despawn` 仅在世界的 hook 注册表非空时才收集组件 id
  的 `Vec`，于是无 hook 的世界完全跳过这次分配（`Vec::new` 不分配内存）。
- 被 blanket allow 掩盖的死代码予以删除（`SpawnColumnBatchIter`、
  `Entities::alloc_many`/`finish_alloc_many`/`resolve_unknown_gen`/`AllocManyState`、
  `Archetype::merge`/`move_to`、`InsertTarget` 类型）；属于有意保留的在建脚手架的
  （entity-ref/component-ref 访问、列批量生成、动态克隆 bundle、面向未来序列化的
  分配器自省）则在条目处带上注释的定点 `#[allow(dead_code)]`。crate 级的
  `#![allow(dead_code)]` 已移除，新的死代码会产生警告。

## Alternatives considered

- **缓存 fetch 而不只是索引（Bevy 的 `QueryState` 形态）。** 暂不采用：fetch
  持有 archetype 列的借用标志，无法存放在 `'static` 的 param state 里；每次迭代
  仍必须重新取借用。按 archetype 缓存列索引可以省去每个 archetype 的列查找，但
  那只是对寥寥几列的二分查找——无界增长的代价在扫描上。
- **按需取借用、完全不存 fetch 列表的迭代器（真正的零分配）。** 否决：它在迭代
  器越过某个 archetype 时就释放该 archetype 的借用标志，而不是持有到 drop，这会
  扩大"已产出的 item 比其列借用标志活得更久"的窗口——正是 fetch 期
  [access registry](../bug-fix/2026-09-19-query-access-registry-and-mainworld-lifetimes.zh.md)
  从 param 侧堵上的那类缺口。在保持现有借用语义的前提下，每次迭代一次精确容量的
  `Vec` 是下限。
- **删掉 edge 表而不是接入。** 否决：接入正是这些移植字段存在的意义，而且收益是
  结构性的，不只是分配次数——一次走透传哈希器的 `(u32, TypeId)` 查找，取代每次
  insert/remove 都构建并哈希整个类型集合键。层级维护（`ChildOf` 的插入/移除）会
  反复命中同样的少数几条边。
- **bundle 插入也放进 `insert_edges`。** 否决：blanket `Component` impl 使元组类型
  也能作为单个组件（`insert_component::<(A, B)>`），于是 bundle 的类型 id 可能与
  组件的类型 id 相同——两个键空间必须分表存放。

## Consequences

- 稳态下的系统查询不再随世界的 archetype 总数增长；每次迭代的代价正比于命中
  集合，每个迭代器只有一次精确容量的分配。
- 结构性变更（`insert_component`/`insert_bundle`/`remove_component`）在重复形状
  下命中一次哈希查找，类型集合的构建每个 (archetype, 组件/bundle) 对只付一次。
- `Query` 按其 `QueryState` 记录的 world id 对应的 world 取参；把一个 state 跨
  world 混用会重建缓存，而不会读到错误的 archetype。
- `Query` 新增的生命周期参数只存在于 `SystemParam` 机制内部（`QueryWindow` state
  变为 `QueryState`）；下游 crate（moonfield-app、moonfield-render-core、
  moonfield-editor）无需改动即可编译。
- 新增测试：系统查询能发现两次运行之间新建的 archetype；带过滤器的查询能命中
  首次运行之后才出现的 archetype；同一个 `SystemState` 跨两个 world 共享时由
  world-id 检查触发重建（system.rs）。

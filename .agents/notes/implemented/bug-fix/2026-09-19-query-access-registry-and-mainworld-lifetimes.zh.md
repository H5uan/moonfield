# Agent Note: Query-param conflict detection and MainWorld lifetimes

Status: implemented

[English](2026-09-19-query-access-registry-and-mainworld-lifetimes.md)

## Problem

`moonfield-ecs` 中的两个健全性漏洞：

1. `QueryIter` 在 drop 时释放 archetype 列的借用标志，但它产出的 item 以
   `'w` 借用 world，比迭代器活得更久。`let refs: Vec<&A> = q1.iter().collect();`
   在标志释放后仍持有 `&A`，同一系统中并列的 `q2: Query<&mut A>` 随后可以调用
   `iter_mut()` 与之别名——同一系统的各个 param 之间没有任何检查。
   （`fetch_mut_cell` 的同类缺口已通过标记 `unsafe` 堵住，见
   [spawn_at hooks and mutable-query safety](2026-09-05-spawn-at-hooks-and-mutable-query-safety.zh.md)；
   而 `Query::iter` 入口仍可从 safe 代码到达。）
2. `MainWorld`——extraction 期间停靠进渲染世界的主世界——暴露
   `unsafe fn world<'w>(&self) -> &'w World`，生命周期由调用方任选，于是一个
   停靠的 resource 可以造出任意多个相互重叠、与 resource 借用脱钩的 `&World`
   引用。

## Decision

- world 携带一个 `AccessRegistry`（按 `TypeId` 键控的逐组件读计数加写集合）。
  `WorldQuery` 新增 `register_access`/`unregister_access`：`Query::fetch` 注册
  该查询的组件访问，`Query::drop` 注销，于是注册表始终精确镜像当前存活的
  param。在存活的写上注册读、或在任何存活访问上注册写，都会在 fetch 时
  panic——先于任何迭代，也先于 item 比标志活得更久。fetch 先把注册写入注册表的
  暂存副本，于是与自身冲突的查询（`Query<(&A, &mut A)>`）panic 时不会留下部分
  注册。检查按组件进行、不看 filter：`Query<&A, With<X>>` 与
  `Query<&mut A, Without<X>>` 即使 filter 不相交也算冲突。
- `MainWorld::world(&self) -> &World` 把返回引用绑定到 resource 借用；
  `world_mut(&mut self) -> &mut World` 覆盖独占情形（目前尚无调用方需要），于是
  `&MainWorld` 只能产出以 guard 生命周期为界的共享借用，`&mut MainWorld` 至多产出
  一个 `&mut World`。`Extract::fetch`（moonfield-render-core）通过裸指针往返把
  guard 绑定的生命周期重新表达为 fetch 生命周期，其正当性由返回值中持有的
  `Ref<MainWorld>` 保证。停靠机制本身不变。
- 已审计 `World::query_filtered`/`query_filtered_mut` 是否有同样的洞：它们接收
  `&self`/`&mut World`，借用检查器已经门控了它们交出的所有访问；无需改动。

## Alternatives considered

- **lending iterator：把每个 item 绑定到 `QueryIter::next` 的 `&mut self` 借用。**
  否决，理由同
  [spawn_at hooks and mutable-query safety](2026-09-05-spawn-at-hooks-and-mutable-query-safety.zh.md)
  所记录：Rust 的 `Iterator` 无法产出借用 `&mut self` 的 item，为此要重做整个
  查询引擎的 lending-iterator 设计——还会破坏调用方依赖的 `iter().collect()`
  模式。注册表堵住了同一个洞，同时保留迭代器 API。
- **按系统运行清空：每次系统运行前清空注册表，而不是在 drop 时注销。** 否决：
  param 也会在 `FunctionSystem::run` 之外被 fetch（测试里直接调
  `SystemParam::fetch`、渲染命令里的 `SystemState::get`），按运行清空覆盖不到
  这些路径，残留的注册会与后续 fetch 误冲突。按 drop 注册的方案统一覆盖所有
  fetch 路径，不存在会漏掉的清理点。
- **感知 filter 的冲突检测（Bevy 的 `With`/`Without` 不相交判定）。** 否决：它
  要求检查时能拿到每个 filter 的组件集合；现有系统没有依赖 filter 不相交的
  param 对，而保守的 panic 是响亮失败，好过静默放行真实的冲突。

## Consequences

- `Query` param 访问不兼容的系统会在 fetch 时 panic（`conflicting Query params`
  消息），先于任何迭代；不冲突的 param 继续使用 `iter().collect()` 模式。
- 从 `&MainWorld` 只能导出以借用生命周期为界的共享 `&World`；可变访问必须经过
  `&mut MainWorld`。
- 注册表按 `World` 隔离：`Extract<Query<…>>` 注册进停靠的主世界，param drop 时
  释放注册，因此顺序运行的 extract 系统互不冲突。
- 新测试：读/写与写/写 param 冲突在 fetch 时 panic、查询内部冲突回滚注册、
  param drop 时释放注册（system.rs），以及 park/world/unpark 往返（world.rs）。

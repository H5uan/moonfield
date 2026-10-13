# Agent Note: spawn_at hook coverage, hook panic safety, and mutable-query safety contract

Status: implemented

[English](2026-09-05-spawn-at-hooks-and-mutable-query-safety.md)

## Problem

`moonfield-ecs` 生命周期与查询机制中的三个正确性缺口，由一次架构评审发现：

1. `World::spawn_at` 覆盖**存活**实体时直接丢弃其旧组件，不触发任何生命周期
   hook。`ChildOf` 的解绑 hook 从不运行，于是父实体的 `Children` 可能悬挂指向一个
   已不再携带 `ChildOf` 的实体——层级不变量被静默破坏。
2. `WorldQuery::fetch_mut_cell`/`fetch_mut_cell_with` 是 safe（尽管
   `#[doc(hidden)]`）的 trait 方法，能从共享的 `&World` 产出 `Mut<'w, T>`。列借用
   标志在**迭代器** drop 时释放，而产出的 item 在 `'w` 内持续有效：collect 这些
   item、drop 迭代器、再次 fetch 同一列，就能在 safe 代码中制造别名 `&mut`。
3. panic 的 hook 会永久丢失：`fire_hook` 运行前把 hook 从注册表中取出（递归防护），
   只在成功路径放回，于是一次 panic 就会注销该 hook——连同它维护的层级不变量。

## Decision

- `spawn_at` 现在在调用 `alloc_at` **之前**为旧组件触发 despawn 序列
  （`on_despawn` → `on_discard`），因为 `alloc_at` 会使实体位置失效（直到
  `spawn_inner` 重新放置该行），而位置失效期间 hook 连读取该实体都做不到。若 hook
  despawn 了该实体，本次 spawn 优雅中止（本 crate 对"hook 修改被 hook 实体"的一贯
  约定）。随后移除旧行（丢弃其值）并按旧组件逐个触发 `on_remove`，与 `despawn`
  对齐；新组件仍由 `spawn_inner` 照常触发 `on_add` → `on_insert`。
- `fetch_mut_cell`/`fetch_mut_cell_with` 改为 `unsafe fn`，契约要求产出的 item 不得
  比"门控列访问的独占借用"活得更久。两个 safe 入口各自给出了健全性论证：
  `fetch_mut` 接收 `&mut World`（item 使该借用保持存活），`Query::iter_mut` 接收
  `Query` param 的 `&mut self`（item 使该 query 借用保持存活）。
- `fire_hook` 用 `catch_unwind` 包裹 hook 调用，先把 hook 放回注册表，再
  `resume_unwind`，于是 panic 的 hook 不再丢失，panic 本身也照常传播。

## Alternatives considered

- **把查询 item 的生命周期绑到迭代器（lending iterator）。** 否决：Rust 的
  `Iterator` 无法产出借用 `&mut self` 的 item，为此要重做整个查询引擎的
  lending-iterator 设计，与一个契约修复不成比例。标记 `unsafe` 保留了 GAT 设计，
  并把不变量（此前只是注释）移入类型系统；该 trait 的演进见
  [query-filters](../feature/2026-08-19-query-filters.zh.md)。
- **保持 `spawn_at` 不触发 hook，仅文档化该限制。** 否决：静默破坏层级链接比
  在覆盖存活实体的路径上付出 despawn hook 序列的代价更糟；`despawn` 本就已承担
  这份开销。
- **用持有 world 原始指针的 RAII guard 放回 hook。** 否决：`catch_unwind` +
  `resume_unwind` 不新增 `unsafe` 即可达到同样的 panic 安全（`panic = abort`
  构建两种方式都不受影响）。

## Consequences

- 经 `spawn_at` 覆盖存活实体会保持层级不变量：覆盖子实体会把它从父实体的
  `Children` 中解绑；覆盖父实体会级联 despawn 其子实体（linked-spawn），与
  `despawn` 语义一致。由 `hierarchy.rs` 和 `hooks.rs` 中的新测试覆盖。
- `Commands::spawn` 路径（reserve + 对保留 id 调 `spawn_at`）行为不变：保留 id
  没有组件，不会触发 despawn hook。
- 误用 `fetch_mut_cell` 现在必须写 `unsafe`；crate 内的调用点都带 SAFETY 注释。
- panic 的 hook 会在 panic 继续传播前被放回注册表，hook 支撑的不变量在 hook
  失败后依然存活。

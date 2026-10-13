# Agent Note: Added/Changed query filters and seed-based transform propagation

Status: implemented

[English](2026-09-19-per-system-change-detection.md)

## Problem

per-system 变更检测窗口（见[窗口 note](2026-09-14-per-system-change-windows.zh.md)）
让每个系统有了自己的 `(last_run, this_run)` 窗口，并让 `Ref`/`Mut` item 能够感知
tick，但没有任何 query 把窗口当作*过滤器*消费：`Added<T>`/`Changed<T>` 不存在，
存储的 per-component tick 只能通过命令式接口观测。与此同时，
`ensure_global_transforms`/`propagate_transforms` 每帧在 `Update` 和 `PreRender`
各走一遍整棵层级树，无条件重写所有 `GlobalTransform`——这正是变更检测存在所要
省掉的 workload。

## Decision

- `QueryFilter` 拆分为 archetype 级与逐行两部分。archetype 级部分
  （`matches_component_set`）不变，仍是 per-system `QueryState` archetype 缓存
  记忆的内容。逐行部分是一个 GAT（`RowState<'w>`），在迭代器构建时按命中的
  archetype 构建，在 `QueryIter::next` 中逐实体求值；`With`/`Without`/`Or` 使用
  `()`，会被内联消除，而 `Added<T>`/`Changed<T>` 持有组件 tick 列指针与查询系统
  的窗口，逐行比较 added/changed tick。因此缓存只记忆 archetype 匹配，从不记忆
  tick 结论。过滤器的组合方式不变：元组取合取，`Or` 取析取，`()` 匹配一切。
- `QueryIter` 泛型化到 `F`（默认 `()`，`World::query` 等既有签名不变）。
  `Query::get` 在 remote 的 `QueryGetGuard` 路径之上同样应用逐行谓词：先定位
  行，对该行求值过滤器的逐行状态，再走通常的逐实体 fetch。tick 过滤器在 fetch
  时把其组件 tick 列的读访问注册进世界的 access registry。
- `propagate_transforms` 在 remote 的组合查询 + `Local` worklist 结构之上改为
  种子驱动（而不是旧的多查询形态）：`seeds` 参数
  `Query<&Transform, Or<(Changed<Transform>, Changed<ChildOf>)>>` 只扫描自传播
  系统上次运行以来变更的实体，组合查询 `nodes` 回答逐实体查找。每个种子的
  global 被重算——根取局部 affine，非根复合到父级*已存储的* global 上——然后
  种子的整棵子树经 worklist 重写，不再做进一步的 tick 检查（链上任意一处变更
  都会移动所有后代）。正确性与顺序无关：若种子的祖先也变更了，祖先的级联会以
  相同结果把种子再算一遍。未变更的子树从不进入。解除 `ChildOf` 链接（唯一不
  会在实体上留下新 tick 的变更）由 `ChildOf` 的 discard hook 把孤儿实体的
  `Transform` 标记为 changed 来覆盖。`ensure_global_transforms` 改为扫描
  `Query<&Transform, Added<Transform>>` 而不是全部 Transform。两个系统在
  `Update` 和 `PreRender` 中的注册保持原名、原顺序，编辑器的
  `editor_prepare.before(&ensure_global_transforms)` 契约不受影响。

## Alternatives considered

- **Bevy 0.20 的脏位传播（`TransformTreeChanged` + `mark_dirty_trees` +
  `RemovedComponents<ChildOf>`）。** 否决：它需要移除追踪（我们没有
  removed-components 通道）以及一个沿祖先链向上传播标记组件的第二个系统。种子
  扫描利用已有 tick 覆盖同样的场景——`Changed<ChildOf>` 捕获挂载与换父，discard
  hook 捕获孤儿。
- **把传播回退到 rebase 前的多查询形态（独立的
  `transforms`/`childofs`/`children`/`globals` 参数）。** 否决：remote 基线已经
  用一个组合查询加 worklist 级联解析每个节点；在该结构上增加一个 `seeds` 参数
  只需维护一种节点布局和一条级联循环，而不是为每个实体重新引入四次查找。
- **种子过滤器中加入 `Added<GlobalTransform>`（与 Bevy 对齐）。** 否决：它会在
  `GlobalTransform` 上注册读访问，与传播系统自身的 `&mut GlobalTransform` 元素
  在 fetch 时冲突（access registry 不会把过滤器的读并入同 query 元素的写之下）。
  `ensure_global_transforms` 上的 `Added<Transform>` 已经覆盖了该成员所要处理的
  生成路径。
- **把过滤器的读并入 query 自身的写访问之下（Bevy 的 `FilteredAccess` 合并），
  使 `Query<&mut T, Changed<T>>` 合法。** 作为没有调用方的机制否决：tick 过滤器
  注册普通读访问，因此 `Changed<T>` 与同 query 中的 `&mut T` 组合会像其他读写
  冲突一样在 fetch 时 panic。工作区中没有系统需要这种形态；传播查询对其过滤的
  组件都是只读的。

## Consequences

- 安静帧的代价从整树遍历重写所有 global 降为每次 schedule 一次种子扫描；
  `GlobalTransform` 的 changed tick 从此表示"位姿确实移动过"——这正是未来提取
  侧变更过滤器所要消费的信号。
- 变更中的树行为不变：同样的运行重算同样的 global，由既有的传播测试以及
  render-core 中驱动 `PreRender` transform 写入的提取测试共同验证。
- tick 过滤查询上的 `Query::get` 应用逐行谓词；命令式的
  `World::query_filtered` 入口对照 world 全局窗口求值 tick 过滤器。
- 已知边界：`Query<&mut T, Changed<T>>` 单 query 组合会在 fetch 时按读写冲突
  panic——予以接受，当前没有调用方需要。
- 新增测试：`Added`/`Changed` 在 world 窗口上的按次投递与 `Or` 组合；窗口不同
  的两个系统看到不同的变更集；逐行 `Query::get`；传播跳过未变更子树（以
  `Changed<GlobalTransform>` 计数系统探查：空帧 0 次写入，移动根只重写其子
  树）、覆盖挂到未变更父级下的新子节点、并恢复被解除链接的孤儿实体。

# Agent Note: Scene load commit semantics

Status: implemented

[English](2026-10-14-scene-load-commit-semantics.md)

## Problem

[架构契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
的第二项：场景加载不具备失败原子性。`load_scene` 逐个解析并应用 root，后面某个 root
无效时，前面 root 的实体已经滞留在目标 world 中。`ResolvedScene::apply` 在可失败的
template 构建完成之前就 spawn 实体，asset 缺失或子 template 失败会留下残缺的子树。

## Decision

**解析是纯的，且先行。** `load_scene` 在任何 spawn 之前解析文档的全部 root（解析、注册
表查询、template 构建）；无效文档因此完全不会触碰 world。

**apply 跟踪自己 spawn 的实体。** `ResolvedScene::apply` 记录它创建的每个实体（递归、按
spawn 顺序），在任何错误返回前按相反顺序 despawn——先子后父，`ChildOf` 的解除链接钩子
因此永远不会遇到已死的目标。`load_scene` 在 root 之间给出对称的保证：某个 root 的 apply
失败时，本次调用已经 spawn 的 root 全部被 despawn（层级 root 的链接子节点随之级联）。

**保证以操作拥有的实体为界。** template 经由 `TemplateContext` 拿到 `&mut World`：它可
以加载 asset、修改 resource、自行 spawn 实体，这些都不回滚。已加载进 world asset 存储的
asset 保持缓存——asset server 的路径缓存使重试无需重新读盘即可复用。

## Alternatives considered

- **完全分离的两阶段提交**（先对着 world 构建所有 template 输出，再统一 spawn）。暂缓：
  `Template::build` 合法地需要 `&mut World` 做 asset 解析，而为第二阶段擦除构建产物，恰恰
  需要限制 template 接口赖以存在的那些副作用。提案本身就把强原子性放在限制这些副作用
  之后。
- **只 despawn 失败子树的 root。** 未采用：`apply` 在每个子树完全应用之后才把它链接到父
  节点，apply 中途失败留下的实体没有任何 root 可达；只有跟踪式清理能覆盖它们。

## Consequences

- 无效的后置 root、缺失的 asset、失败的子 template 都不会留下操作拥有的实体；该行为由
  `moonfield-scene` 的测试钉住（`resolved.rs` 覆盖 apply 保证，`file.rs` 覆盖整个文档的
  加载，包括失败时的 asset 缓存保留）。
- 向同一 world 重试失败的 `load_scene` 是安全的：实体不会重复，已加载的 asset 从路径缓存
  直接复用。
- 需要更强原子性（resource 回滚、template 自行 spawn 的实体）的调用方暂不具备该能力；
  那需要受限的 template 副作用契约，而不是更多清理。

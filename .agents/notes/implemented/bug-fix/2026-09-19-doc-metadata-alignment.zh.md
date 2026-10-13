# Agent Note: doc and metadata alignment fixes

Status: implemented

[English](2026-09-19-doc-metadata-alignment.md)

## Problem

一次对持久文档的检查发现四处与已交付代码不符：

1. 根 `AGENTS.md` 的名录对 `moonfield-ecs` 的描述没有包含它承载的
   Transform/GlobalTransform 传播（`hierarchy.rs`），尽管
   [docs/architecture.md](../../../../docs/architecture.md) 已经记录了传播系统以及刻意
   保持的 `moonfield-ecs` → `moonfield-math` 依赖方向。
2. `moonfield-math` 的 `Cargo.toml` 带着工作区里唯一一条中文清单注释；其余所有
   `Cargo.toml` 注释都是英文（双语配对只适用于 Agent Notes)。
3. `sphere_from_points(&[])` 会静默返回球心为 NaN 的包围球，而同文件的
   `aabb_from_points` 对空输入返回 `None`——这一分歧没有任何文档说明。
4. 元组和 `Option` 查询形状触发 `Query::get` panic 时，消息既没指出是哪个形状不受
   支持，也没指出可用的替代用法。

## Decision

只修文档，不改行为。`AGENTS.md` 名录行现在写明了传播功能和 math 依赖。清单注释已
翻译为英文。`sphere_from_points` 的文档写明：空切片会让零和除以（为零的）点数，得
到球心各分量为 NaN、半径为 `0.0` 的包围球。`Query::get` 的 panic 消息现在包含不受
支持形状的类型名，并指向 `World::get_component`/`World::get_component_mut` 和迭代查
询这两条可行路径。`docs/architecture.md` 中关于 `spawn_batch` 和 `World::clear` 不触
发钩子的论断已对照 `world.rs` 复核，保持原文不变。

## Alternatives considered

- **让 `sphere_from_points` 对空输入返回退化但有限的包围球。** 否决：编造哨兵值和
  NaN 一样会掩盖调用方的错误，而改成 `Option<BoundingSphere>` 属于破坏性的签名变更；
  记录 NaN 契约遵循了
  [doc-claims-vs-code audit](2026-09-19-doc-claims-vs-code-audit.md) 确立的规则——
  描述已交付的行为，而不是顺手重新设计它。
- **为元组和 `Option` 形状实现 `Query::get`。** 否决：`WorldQuery::get_entity` 的文
  档约定在有调用方需要时再移植这些形状，目前没有任何调用方需要，所以 panic 是正确
  的失败方式，只需改进它的消息。

## Consequences

- 根名录、math 清单和 bounding/query 的文档与代码一致；运行时行为没有任何变化。
- 在不受支持的形状上调用 `Query::get` 触发的 panic 现在会报告该形状的类型名和支持
  的替代用法，不读 `query.rs` 也能据此修正代码。

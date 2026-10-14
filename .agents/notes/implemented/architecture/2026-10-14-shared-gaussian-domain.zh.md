# Agent Note: Shared Gaussian domain crate

Status: implemented

[English](2026-10-14-shared-gaussian-domain.md)

## Problem

[架构契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
的第五项：训练器（`moonfield-ml`）从 `moonfield-render-feature` 导入 `GaussianScene`，把
app/render 框架拖进了一个只需要 CPU 域数据的消费者。`GaussianScene` 的平行公开向量可能长
度不一致，而 `pack_gaussians` 会静默截断到最短属性。glTF 加载器的 `ln`/`logit` 转换也可
能从退化文件数据（零 scale、0/1 opacity）造出非有限值。

## Decision

**`moonfield-gaussian` 拥有该域。** 新的叶子 crate（无框架依赖）承载 `GaussianScene` 和
`SH_REST_LEN`；渲染器与训练器都消费它，`moonfield-ml` 完全去掉对
`moonfield-render-feature` 的依赖。crate 划分跟随依赖图：scene 类型是唯一的共享概念，因
此它拥有自己的 crate，而不是寄居于任一消费者。

**有效性是构造期不变量。** `GaussianScene` 的字段是私有的；`new` 和 `push` 校验属性长度
一致与数值有限（`GaussianSceneError::{LengthMismatch, NonFinite}`），访问器只读。长度不
一致的场景无法被表示，因此 `pack_gaussians` 精确打包 `scene.len()` 条记录——截断路径不
复存在。

**加载器在空间转换前夹紧。** `SCALE` 在 `ln` 之前夹到正的最小值，`OPACITY` 在 `logit`
之前夹进 `[1e-6, 1 - 1e-6]`，文件数据因此无法造出场景现在会拒绝的非有限值；场景层的失
败以 `SplatGltfError::Domain` 浮现。

## Alternatives considered

- **把类型留在 render-feature，让 ml 继续依赖它。** 未采用：这正是要修复的耦合——训练器
  会为一个纯数据类型继续编译整个 app 框架。
- **公开字段 + 仅在 pack 时校验。** 未采用：构造点是最便宜的拒绝位置（glTF 加载器的错误
  因此指向文件，而不是 GPU 侧的症状），而公开字段让每个未来的写入者都能破坏不变量。
- **数值化校验表示约定**（log-space scales、logit opacities、(w, x, y, z) rotations）。
  放弃：这些约定在数值上无法与其 render-space 对应物区分；它们保持为文档化的、由调用方
  维护的契约。

## Consequences

- 渲染器与训练器消费同一个经过校验的接口；长度不一致的属性数组在 `new`/`push` 被拒绝，
  而不是在 pack 时被截断。
- `moonfield-ml` 不再编译 render 框架；编辑器与测试通过校验过的构造器构建场景。
- `0.282095·c + 0.5` 的 SH 偏置与 kernel 约定仍归
  [splat forward 决策](2026-10-14-splat-forward-pass.md)所有；本 note 只拥有数据的归属与
  有效性。

# Agent Note: Editor action interface

Status: implemented

[English](2026-10-14-editor-action-interface.md)

## Problem

[架构契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
的第六项：编辑器 UI 回调直接对 `World` 执行资产加载、场景 I/O 和 Transform
编辑，撤销、后台导入和自动化因此需要分别实现同一种操作，且操作本身没有
校验或结果契约。

## Decision

**`EditorAction` 是唯一的执行接口。** `moonfield-editor` 的 `action`
模块拥有校验、结果和撤销记录。`execute` 执行一个动作并返回
`ActionOutcome`（状态消息、受影响实体、逆动作）；`perform` 执行并把逆动作
记录进 `EditorHistory` 世界资源；`record` 把一个已经生效的可逆手势收成一个
撤销单元。操作集合为 `ImportAsset`、`LoadScene`、`SaveScene`、
`SetTransform`、`DeleteEntity` 和 `Batch`；`Batch` 在后续步骤失败时回滚
已完成的步骤。

**所有 UI 路径都经由该接口。** 内容与场景面板调用 `perform`；Ctrl/Cmd+Z
与 Ctrl/Cmd+Shift+Z（或 Ctrl/Cmd+Y）遍历历史。gizmo 拖拽为即时反馈逐帧
直接写入，手势结束时记录一个撤销单元（拖前 Transform → 结束
Transform）。渲染帧交接不在该接口之内。

**无窗口测试执行相同操作。** `action` 单元测试在无显示环境下驱动
`perform`/`undo`/`redo`：导入和场景加载可逆、失败导入不记录任何内容、
`SetTransform` 校验实体与组件、批次回滚、已记录的手势撤销回起始状态。

## Alternatives considered

- **在 `World` 层做撤销。** 舍弃：实体被销毁后，组件变动没有有意义的逆
  操作，通用撤销层也无法命名“加载场景”这类操作。动作层把操作与其逆操作
  放在一起拥有。
- **每拖拽帧记录一个撤销单元。** 舍弃：一次拖拽会向历史写入大量几乎相同
  的条目；`record` 把整个手势收成一个单元，逆操作是拖前 Transform。
- **带延迟执行的命令队列。** 推迟：目前没有任何跨帧排队的动作。后台导入
  会建立在该接口之上；在那些消费者出现之前设计队列只会猜测需求。

## Consequences

- UI 代码只负责展示（状态文本、选中状态）；撤销所需的一切变更都经由
  动作接口。
- `DeleteEntity` 刻意不可逆——销毁会丢弃组件，逆操作必须快照并重建整棵
  子树。
- inspector 的 reflect 通用编辑直接写组件；撤销恰好覆盖上述操作集合。

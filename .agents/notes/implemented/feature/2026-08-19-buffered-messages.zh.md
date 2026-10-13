# Agent Note: 缓冲消息（Message/Messages/读写参数）

Status: implemented

[English](2026-08-19-buffered-messages.md)

## Problem

引擎需要参考实现中的缓冲事件机制（在其当前开发分支中已更名为
*messages*）：写入者向通道压入值，多个系统各自消费一次，并自动清理——
用以替代此前手写的按帧清理队列（`WindowEvents`、`RawWindowEvents`），
后者需要窗口后端和编辑器在每帧末尾手动清空。路线图早已明确把
`WindowEvents` 通道列为该机制的迁移目标。

## Decision

`moonfield-ecs` 新增 `message.rs`，是对参考实现 `bevy_ecs::message` 的
架构级移植：

- `Message` 是 blanket 实现的标记 trait（`Send + Sync + 'static`），与
  本仓库的 `Component`/`Resource` 一致——本工作区不引入 derive。
- `Messages<M>` 是双缓冲存储 resource：`write` 以单调递增的 `MessageId`
  追加到当前缓冲区；`update()` 交换两个缓冲区并清空较旧者，使每条消息
  存活两帧（每帧都读的读者永不丢消息；跳一帧可能丢；跳两帧旧消息必然
  被丢弃）。
- `MessageCursor<M>` 是每个读者自己的状态；`MessageReader<M>` /
  `MessageWriter<M>` 是系统参数（读者的 cursor 即其
  `SystemParam::State`）。
- `App::add_message::<M>()` 插入 resource 并把该类型注册进
  `MessageRegistry` resource；`message_update_system`（独占系统）每帧在
  新增的 `First` schedule 中交换所有已注册缓冲区，`App::update` 在
  `Update` 之前运行 `First`。

迁移：`WindowEvents` 与 `RawWindowEvents` 已删除。winit 后端把
`WindowEventKind` 和原始 winit `WindowEvent` 写入 `Messages<…>`
resource；编辑器（独占渲染系统）在 `EditorState` 中保存
`MessageCursor<WindowEvent>`，把新的原始事件喂给 egui。`InputState`
保持锁存语义不变（参考实现同样把按键输入状态与消息流分开）；其内部
事件重放队列**没有**迁移——除自身测试外没有消费者，迁移它只是无收益
的改动。

最小移植偏差（已在模块文档中记录）：参考实现基于 change tick 跳过未变
更缓冲区的优化、以及面向定步更新的信号机制未移植（我们的 resource 没有
逐 resource 的 change tick）；缓冲区每帧无条件交换，对每帧读取的读者而
言可观察语义完全一致。

## Alternatives considered

- **保留按帧清理队列、与消息并存。** 否决：两套重叠的事件机制容易行为
  漂移；本次的目的就是替换手写清理模式。
- **把 `InputState` 的内部事件队列也迁移。** 暂不迁移：没有消费者读取
  它，而且 pressed/just_pressed 锁存语义与消息流是两种不同契约。若将来
  出现 gameplay 消费者再议。
- **引入类型擦除的逐 resource change-tick 跟踪以跳过未变更的交换。**
  否决，属于过早优化：交换不过是每个已注册类型每帧一次 `Vec::clear`；
  参考实现的该优化服务于其并行执行器。

## Consequences

- 任何事件式通道现在只需一次 `app.add_message::<T>()`；读者免费获得按
  系统的游标。
- 若消息类型未注册，消息参数在 fetch 时 panic（与 `Res<T>` 缺失
  resource 的策略一致）；panic 信息会指明 `App::add_message`。
- `App::update` 现在先运行 `First` 再运行 `Update`；用户系统也可以排入
  `First`。
- 消息不再在帧末清空：从不运行的读者最多留下两帧消息，随后静默丢弃——
  与参考实现语义一致。

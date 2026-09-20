# Agent Note: run_schedule merges mid-run registrations

Status: implemented

[English](2026-09-19-run-schedule-merges-mid-run-registration.md)

## Problem

`World::run_schedule` 把正在运行的 schedule 条目从 `Schedules` resource 中取出，运行
完毕后再插回。若运行期间某个系统对*同一* label 调用 `World::add_systems`（或
`add_sets`），注册会走 `Schedules::entry`，它创建一个全新的空 schedule；运行结束时的
回插会覆盖该条目——新注册的系统被静默丢失。

## Decision

`Schedule` 新增 `merge_from(&mut self, other: Schedule)`：追加 `other` 的系统（针对本
schedule 的 set 链重新展开 `in_set` 归属，且不重复 config 已携带的约束）并吸收 `other`
的 set 链，然后将顺序标记为 dirty，使下一次运行重新排序。`run_schedule` 在运行结束后
检查 `Schedules` resource 中是否存在运行中 label 的条目——这只可能意味着某个系统在
运行期间向其注册——并把该条目合并进即将回插的 schedule，而不是覆盖。注册的系统从下
一次 `run_schedule` 调用开始运行，照常按其 `before`/`after` 约束排序。
`Schedule::add_systems` 与 `merge_from` 共用新的 `push_config` 辅助函数，set 锚点展开
只有一份实现。

## Alternatives considered

- **当运行期间的注册指向运行中的 label 时 panic。** 否决：schedule 的数据模型（有序
  config 列表加稳定拓扑排序）天然支持合并——运行期间的注册不过是一次迟到的注册——
  而且本 schedule 层移植自 Bevy，Bevy 对运行中 schedule 新增系统的做法是在下一次运行
  时生效，而不是报错。
- **在 world 上缓冲运行期间的新增，由 `run_schedule` 统一应用。** 否决：这会引入一条
  平行的注册通道（`World` 需要一个自带生命周期的待注册字段），而 `Schedules` 中那个
  新建的条目恰好已经持有全部新增内容，类型与约束俱全。

## Consequences

- 向运行中的 label 注册有了明确定义：新增内容被合并，从下一次运行起生效；不再有静默
  丢弃。
- 每次运行都无条件注册的系统会让它的 schedule 无限增长（每轮都追加）——是否重复注册
  由调用方自行控制；回归测试正是利用这一点来证明合并生效。
- 从运行中的 schedule 内部重跑同一 label 仍是 no-op（条目已被取出），行为不变。
- 新测试：一个系统在运行期间向自身 label 注册一个带约束的系统，该新增在下一次
  `run_schedule` 中按约束顺序运行（schedule.rs）。

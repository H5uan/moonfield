# Agent Note: Per-system change-detection windows

Status: implemented

[English](2026-09-14-per-system-change-windows.md)

## Problem

变更检测的模块文档承诺每个系统拥有自己的 `(last_run, this_run)` 窗口，但查询迭代实际比较的是世界全局窗口 `(last_change_tick, change_tick)` —— 最近一次 tick 推进的窗口，由所有系统共享，且 tick 每个 schedule run 只推进一次。由此产生两类漏检：

- 间歇运行的系统（帧率高于步长的 [fixed-timestep](../feature/2026-08-20-fixed-update.zh.md) schedule）比较的窗口已经越过了它空闲期间发生的写入。
- 同一个 schedule run 内所有系统共享一个 tick，排在读取者*之后*的系统写入落在读取者自己的 `this_run` 边界上，读取者的下次 run 永远看不到。

`CHECK_TICK_THRESHOLD` 的钳位有文档但从未被调用；schedule 模块文档声明的"每 run 推进一次"也与 per-system 窗口的文档互相矛盾。

## Decision

一次 system run = 一个 tick，且每个系统持有自己的窗口：

- `FunctionSystem::run`、`ExclusiveSystem::run` 和 `SystemState::get` 在入口推进世界的变更时钟。`World::increment_change_tick` 返回本次 run 的 tick，并把计数器留在下一个 run 的位置上，因此 run 之后的写入 —— 更晚的系统、应用中的 commands、世界访问器 —— 记录的 tick 严格新于该 run 观察到的一切。
- 每个 `FunctionSystem` 记住自己的 `last_run`（最近一次完成的 run 的 tick），并以窗口 `(last_run, this_run)` 取参数。首次 run 的 `last_run` 为 0、时钟从 1 起，因此首次运行观察到所有已存在的组件都是新的。
- 窗口经由 `SystemParam::refresh_window(state, last_run, this_run)` 传递 —— 默认空实现，tuple 逐项转发。`Query` 参数的状态即窗口（`QueryWindow`），其迭代器与逐实体访问携带这两个 tick。`World::query*` 与 `get_component_mut` 保持世界窗口。
- 时钟字段为内部可变（`Cell`），使 `SystemState::get` 能通过共享的世界借用推进时钟。
- schedule 的 set 锚点既不观察也不写入，不推进时钟。
- [Extract schedule](2026-09-09-extract-schedule.zh.md) 的 `Extract<T>` 以*主*世界的时钟度量内部参数的窗口（`ExtractState { inner, last_run }`）：主世界中自该参数上次 fetch 以来的变更。渲染 schedule 的窗口不跨世界适用。
- `Schedule::run` 以 `World::check_change_ticks` 开场，按 `CHECK_TICK_THRESHOLD` 相对上次通过限流：钳位每个 archetype 的 tick 行和每个已存储 schedule 的系统 `last_run`（`System::check_change_ticks`），使 `u32` 时钟回绕后比较保持确定性。

机制与参考实现的当前形态（0.20-dev）一致：tick 在系统内部推进、返回值即本次 run 的 tick、`SystemState` 每次 fetch 推进、周期检查同时遍历组件与系统的 tick。

## Alternatives considered

- **在每 schedule run 一个 tick 的基础上加 per-system `last_run`。** 改动更小的方案：修复间歇 schedule 的漏检，但同 run 的晚写者仍记录共享 tick，逃过读取者的下个窗口。每 run 一个 tick 同时关掉两者。
- **让 `SystemState::get` 只观察（不推进）。** 中间无系统 run 的连续 get 共享一个窗口、互相看不到对方的写入；渲染命令逐 phase item 取参数。
- **只钳位组件 tick。** 长期不运行的 schedule 持有古老的 `last_run`；回绕后其窗口超过 `MAX_CHANGE_AGE`，所有比较都报告为已变更。两侧同时钳位让古老比较保持确定性。

## Consequences

- `Schedule::run` 推进时钟的量为运行的系统数；空 schedule 不推进。
- 系统看到自它上次 run 以来的全部写入 —— 包括同 run 中更晚系统的写入、其后应用的 commands、run 之间的世界访问器写入 —— 且永远不会重复看到自己的写入。
- 世界窗口 `(last_change_tick, change_tick)` 覆盖最近一次 tick 推进之后的世界访问器写入。
- `EntityMut` 增加了 `is_added`/`is_changed`：它的 `Deref` 直接指向组件，`Mut` 的访问器此前从逐实体访问无法触达。
- 时钟从每 schedule run 一次改为每 system run 一次推进（外加每次 `SystemState` fetch 一次）；`CHECK_TICK_THRESHOLD` 钳位约束随之而来的老化。

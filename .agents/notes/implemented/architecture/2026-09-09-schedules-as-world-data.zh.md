# Agent Note: Schedules as world data

Status: implemented

[English](2026-09-09-schedules-as-world-data.md)

## Problem

schedule 存储是 `App` 的字段——每个 world 一张 map——因此没有系统能运行 schedule：[render pass schedule redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) 的逐 view 执行与 extract schedule 两步都缺地基。参数状态（`Local` 一类）只能在函数系统内部触达，而 exclusive system 与即将到来的 render command 需要在单个系统之外使用 init/fetch 这一对。

## Decision

- `Schedules` 是 `HashMap<TypeId, Schedule>` 资源。`World::add_systems` 向它注册（按需创建资源）；`World::run_schedule(label)` 是唯一的运行原语。
- `World::run_schedule` 只把目标条目取出运行、结束后放回。资源本身留在 world 里，所以 schedule 内部的系统可以运行*其他* schedule；从自己的运行内部重跑同名 label 会发现条目不在，等于 no-op。
- `App::add_systems` / `add_render_systems` / `run_schedule` / `run_render_schedule` 是两个 `World` 方法的包装。`App` 私有的 map 消失，fixed-timestep 循环的闭包改为对每个 label 调 `world.run_schedule`，不再拆借 map。
- `SystemState<P>` 持有一个 `SystemParam` 的持久状态——`new` 初始化、`get` 取一次运行的参数——与函数系统内部持有的是同一对。

## Alternatives considered

- **把 map 留在 `App` 上，给未来的 camera driver 递一个嵌套运行钩子。** 落选：schedule 存储会出现两条访问路径（App 字段与系统可见资源），driver 也要跑在定制回调上而不是唯一原语上。
- **运行时把整个 `Schedules` 资源移出。** 落选：资源在运行期间不在 world 里，内部系统就无法运行别的 schedule；只取条目让资源始终在场——正是 Bevy `try_schedule_scope` 的形状。
- **同名重跑报错（Bevy 的 `TryRunScheduleError`）。** 落选：当前所有调用方都把"schedule 不存在"当 no-op（fixed 循环会跑可能为空的 label），而重入运行只可能是 bug，错误类型帮不上忙。

## Consequences

- 运行期间向该 schedule 注册的系统会在放回时被覆盖；注册属于 plugin 构建期，与当前所有调用方一致。
- `moonfield-ecs` 增加公开表面：`Schedules`、`World::add_systems`、`World::run_schedule`、`SystemState`。
- exclusive system（`FnMut(&mut World)`）不变；driver 系统由它加捕获的 `SystemState` 组合而成。

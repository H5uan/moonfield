# Agent Note: 固定步长 schedule（Time<Fixed> + FixedMain 循环）

Status: implemented

[English](2026-08-20-fixed-update.md)

## Problem

依赖帧率的逻辑会漂移：`Update` 里的系统拿到的是显示节奏给出的任意
delta。物理、模拟以及一切累积式逻辑都需要一个按固定增量前进的时钟，
以及一个每帧运行 0、1 或 N 次来追平的 schedule——即参考实现的固定步长
机制。此前它被推迟，正是因为还没有驱动它的固定更新 schedule。

## Decision

`Time<Fixed>` 落地于 `moonfield-time`（`Fixed` 上下文：`timestep` +
`overstep`；`from_hz`/`from_seconds`/`from_duration`、`set_timestep*`、
`overstep(_fraction)`、`accumulate_overstep`/`discard_overstep`、
`expend`），并提供 `run_fixed_main_schedule`：累积虚拟时间的 delta，
然后按整步长消费，每步运行一次固定 schedule，迭代期间把泛型 `Time`
resource 镜像为 `Time<Fixed>`（结束后恢复为虚拟时间）。

`App::update` 依次运行 `First`、固定步长循环、`Update`。固定侧落地了完
整的 label 集合以保持 API 对齐——`FixedFirst`、`FixedPreUpdate`、
`FixedUpdate`、`FixedPostUpdate`、`FixedLast`，外加 `FixedMain` 伞形
label（直接注册在它下面的系统在每个迭代内、五个子 schedule 之后运
行）。没有 `TimePlugin` 就没有 `Time<Fixed>` resource，循环成为
no-op，因此编辑器路径与无头测试无需任何时间设置。

顺带的一次结构调整：`TimePlugin` 从 `moonfield-time` 移入
`moonfield-app`（与 `HierarchyPlugin` 并列），依赖方向翻转为 app →
time。固定循环必须由 `App::update` 驱动，而它需要时间类型——若插件留
在 time crate，依赖就成环了。`moonfield-time` 现在是纯时钟 crate（唯一
依赖是 `moonfield-ecs`，因为驱动函数要触碰 `World`）。winit 后端不做任
何固定步长专用的输入锁存；固定系统读取同样的逐帧 `InputState`。

与参考实现的偏差（已记录在模块文档中）：`expend` 是公开的（参考实现把
它设为 crate 私有，因为其驱动函数就在 crate 内）；没有
`RunFixedMainLoop` 系统集间接层——驱动是 `App::update` 中硬编码的一
步，因为我们的 schedule 没有"Main 中的系统位"这一概念。

## Alternatives considered

- **把驱动做成 `FixedMain` schedule 里的独占系统。** 否决：我们的
  schedule 存在 `App` 里而不在 world 里，系统无法运行嵌套 schedule；参
  考实现能做到只是因为它的 schedule 是 world resource。把 schedule 表
  搬进 world 是一个大得多的重构，当前没有收益。
- **`TimePlugin` 留在 `moonfield-time`，用 resource 把循环驱动做类型擦
  除。** 否决：为一个本质上是帧相位硬编码步骤的东西引入函数指针注册
  表，只是为了绕开一个完全自然的依赖方向（app 负责组合，time 提供时
  钟）。
- **固定步长放在 `Update` 之后运行。** 否决：参考实现在 `Update` 之前
  运行，使 `Update` 系统看到固定步之后的世界状态；这个顺序是可观察的
  （例如先固定物理、后渲染插值），照抄没有成本。

## Consequences

- 帧率无关的逻辑有了归宿：`app.add_systems(FixedUpdate, …)`，配合
  `Res<Time<Fixed>>`（或在固定运行期间读 `Res<Time>`——此时它就是固定
  时钟）。
- 暂停与变速会传导：虚拟时钟暂停则 delta 为零，固定步长为零。
- `moonfield-time` 依赖很轻，`moonfield-app` 是引擎插件的组合点——与
  `HierarchyPlugin` 同一模式。
- 基于 `overstep_fraction` 的固定步间插值已经可行，但还没有消费者。

# Agent Note: App runner 与帧 tick 对齐 Bevy

Status: implemented

[English](2026-08-27-runner-and-tick-aligned-to-bevy.md)

## Problem

`moonfield-app` 的 `App::run` 有两条循环路径——内置自旋循环(`run_updates`)和可选的插件 runner——而 winit runner 在 `run_frame` 里手工粘合了帧的各个步骤(`update_time`、`sync_windows`、`App::render`、`input.end_frame`)。runner 知道渲染的存在;`App::render` 是独立方法,测试和 runner 都得记得调用;不带 runner 的 `App::run` 会无限循环;在真正的循环旁边还躺着一条基本是死代码的循环。Bevy 的结构不同:`App::run` 总是委托给 runner(默认 `run_once`),循环是 runner 的职责,渲染是主 tick 的一部分。

## Decision

- `App::run` 总是调用 runner;默认是新的 `run_once`,只跑一次 `App::update` tick。`run_updates` 保留为 headless 循环,但不再是 `run` 的兜底。
- `App::update` 是完整 tick:`First` → 固定循环 → `Update` → `render()` → `Last`。新增 `Last` 阶段承载帧末杂务。
- runner 签名现在返回 `AppExit`(镜像 Bevy 的 `RunnerFn`):`set_runner(impl FnOnce(&mut App) -> AppExit)`。`AppExit` 携带 `std::process::ExitCode`(SUCCESS/FAILURE/from_code),同时仍是"插入即退出"的资源。`moonfield-editor` 的 `main` 把它作为进程退出码返回。
- winit runner 的 `run_frame` 现在是 `update_time(...)`(帧边界推进时钟)+ `app.update()`;`sync_windows` 和新加的 `input_end_frame` 移入 `Last` 系统。

时钟推进最初保留在帧边界(runner),因为固定步长测试通过 `Time<Virtual>::advance_by` + `App::update` 确定性驱动时钟。此后它已借 Bevy 的 `TimeUpdateStrategy` 移入调度——见 [2026-08-27-time-update-strategy.zh.md](2026-08-27-time-update-strategy.zh.md)。

## Alternatives considered

- **把 `update_time` 移进 `App::update`。** 否决:会覆盖固定步长测试里确定性推进的时钟。
- **本次提交就引入 `TimeUpdateStrategy`。** 延后到后续提交(现已落地——见 time-update-strategy note):它属于 `moonfield-time` 的独立 API 改动;本次保持机械性。
- **保留 runner 为 `FnOnce(&mut App)`。** 否决:退出码属于 runner 契约,与 Bevy 一致。

## Consequences

- runner 是唯一的循环途径;不带 runner 的 `App::run` 只跑一次 tick 然后返回。
- 任何 runner 都能自动获得时间推进、渲染、窗口同步和输入清理;时间推进此后已通过 `TimeUpdateStrategy` 移入 `First` 系统。
- `App::render()` 保持公开,供测试和嵌入使用;它是 `update()` 的尾部。
- `run_frame` 不再有手工编排顺序——帧的步骤现在由调度声明。
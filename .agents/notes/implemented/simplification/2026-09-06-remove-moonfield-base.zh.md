# Agent Note: remove moonfield-base and unused manifest edges

Status: implemented

[English](2026-09-06-remove-moonfield-base.md)

## Problem

`moonfield-base` 是个残留物:十五行代码,只有 `initialize()` / `shutdown()`,所
做的只是翻转一个名叫 `LOGGING_INITIALIZED` 的原子量——没有任何代码读取它,它也不
守护任何东西。`App::startup`/`App::shutdown` 调用它纯属惯性。另有三条清单边不承载
任何代码:`moonfield-ecs` 依赖 `serde`(零引用)和 `moonfield-base`(零引用),
`moonfield-winit` 在时钟推进迁入 `moonfield-app` 的 `First` 调度(见
[TimeUpdateStrategy](../architecture/2026-08-27-time-update-strategy.md))之后仍依赖
`moonfield-time`。

## Decision

删除 `crates/moonfield-base` 及其在 `moonfield-app::App` 中的两个调用点——原子量
不守护任何东西,因此 `moonfield-app` 里不需要替代物。删除三条未使用的清单边
(`ecs → serde`、`ecs → base`、`winit → time`)。将该 crate 从根 `Cargo.toml` 的
workspace 依赖以及根 `AGENTS.md`、`crates/AGENTS.md` 和
[log 分层笔记](../architecture/2026-09-05-log-crate-layering-boundary.md)的示例列表
中的名录里移除。

## Alternatives considered

- **保留 `moonfield-base` 作为未来共享基元的归属。** 否决:空 crate 只有在有内容时
  才有存在价值;未来出现基元时一次提交即可重建。
- **把 `initialize`/`shutdown` 并入 `moonfield-app`。** 否决:这两个函数唯一的效果
  是翻转一个无人观测的原子量;并入 `App` 保留的是一个空操作,而不是行为。

## Consequences

- 工作区减少一个 crate;`moonfield-ecs` 的依赖边收敛到 `moonfield-math` 与外部
  crate(`thiserror`、`foldhash`)。
- `App::startup`/`App::shutdown` 现在只做初始化标记和运行各自的调度。
- 无行为变化;全工作区测试套件原样通过。

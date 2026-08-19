# Agent Note: Redraw-driven frame loop and pacing

Status: implemented

[English](2026-08-19-render-loop-pacing.md)

## Problem

winit 应用必须决定何时产出一帧:持续渲染会浪费能源/闲置 GPU,而完全在事件之间休眠又可能在连续场景(编辑器重绘、动画)中丢失输入或 UI 更新。后端还需要一个确定的位置来执行每帧的 ECS 与渲染工作,以及一种从循环内部退出以支持自动化冒烟测试的方式。

## Decision

帧循环是 **redraw 驱动**的:`App::about_to_wait` 只决定 `ControlFlow` 并请求重绘;整帧(`App::update` → `sync_windows` → `App::render` → 帧状态清理 → 退出检查)在 `WindowEvent::RedrawRequested` 内运行。

- 节奏由 `WinitSettings` resource 控制(`focused_mode` / `unfocused_mode`:`UpdateMode::Continuous` 或 `Reactive { wait, react_to_* }`,预设 `game()` 默认 / `desktop_app()` / `continuous()`),每次帧决策时重新读取,系统可运行时修改。
- 空闲的 Reactive 循环可由外部线程和 UI 工具包通过 `EventLoopProxyWrapper` resource 唤醒(`wake_up()`,发送 `WinitUserEvent::WakeUp`)。
- 冒烟测试退出:设置 `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` 会在渲染 N 帧后通过共享的 `WindowControl` 触发退出,从而在带显示器的机器上无头测试启动与关闭。

## Alternatives considered

- **无条件持续渲染。** 拒绝:空闲时浪费 CPU/GPU 和电量;`Reactive` 模式正是为规避这一点而存在。
- **在其他 winit 事件内运行帧(如 `MainEventsCleared`)。** 拒绝:把工作绑定到任意事件会把节奏与后端的事件分发耦合;`RedrawRequested` 是 winit 认可的每帧钩子,且符合请求-绘制契约。
- **单独的 `WindowRequests::exit` 通道。** 拒绝:退出是共享 `WindowControl` 的生命周期职责;经第二个通道路由会把退出策略一分为二。

## Consequences

- 依赖帧时间的系统必须锚定到 `App::update` / `App::render`,绝不要直接锚定 winit 事件——某些平台上事件可能在单帧内多次到达。
- Reactive 是默认姿态;任何逐帧动画或重绘要么设置 `WinitSettings::continuous()`,要么发送 `wake_up()` 保持活跃。
- `MOONFIELD_EDITOR_AUTO_CLOSE` 是测试接口,不是产品特性;编辑器之外保持零成本、无副作用。
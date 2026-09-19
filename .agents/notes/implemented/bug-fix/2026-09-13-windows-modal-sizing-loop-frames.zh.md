# Agent Note: Driving frames through the Windows modal sizing loop

Status: implemented

[English](2026-09-13-windows-modal-sizing-loop-frames.md)

## Problem

在 Windows 上拖动窗口边缘时，整个拖动期间窗口没有渲染内容——窗口保持空白
（合成器背景），直到松开鼠标。帧循环是重绘驱动的：`about_to_wait` 请求重
绘，`WindowEvent::RedrawRequested` 通过 `App::update` 跑一个 tick。当用户
缩放或移动窗口时，Windows 在 `DefWindowProc` 内部运行模态消息循环
（`WM_ENTERSIZEMOVE`…`WM_EXITSIZEMOVE`）；winit 的消息泵永远到不了
`about_to_wait`，而且——与 winit 的 `CS_HREDRAW | CS_VREDRAW` 类样式所暗
示的相反——`WM_PAINT` 在模态循环内也不会被派发，`RedrawRequested` 完全
停止到达。

用程序化复现（post `WM_SYSCOMMAND(SC_SIZE)` 加键盘输入可进入真实模态循
环）实测：约 1.8 秒的模态 sizing 循环期间编辑器总共只跑了约 15 帧（应为约
108 帧），`about_to_wait`/`RedrawRequested` 为 +2/s，而正常为 +60/s。
[交换链退休改造](../architecture/2026-09-13-deferred-swapchain-retirement.md)
已经使每 tick 重建的代价足够低；缺失的一环是期间根本没有 tick 在跑。

## Decision

经典的 `SetTimer` 方案，收敛在 Windows 专属模块
（`crates/moonfield-winit/src/sizing.rs`）中：

- `resumed` 对每个 winit 窗口做子类化（`SetWindowSubclass`）。子类过程在
  `WM_ENTERSIZEMOVE` 时启动 16ms 窗口定时器，在 `WM_EXITSIZEMOVE` 时停止，
  在 `WM_NCDESTROY` 时自拆卸；其余消息一律转发给 `DefSubclassProc`
  （winit 的过程，其自身的 `MARKER_IN_SIZE_MOVE` 簿记不受影响）。
- 模态循环会派发 `WM_TIMER`，因此每个定时器 tick 跑一帧。该帧与正常帧走
  *同一个*入口：`WinitHandler::run_modal_frame` 与 `run_frame` 一样调用
  `self.app.update()`，并做相同的 `last_frame`/`redraw_pending` 簿记。只有
  退出请求检查被推迟到下一次 `about_to_wait`（定时器里拿不到
  `ActiveEventLoop`）。
- 子类过程经由一个线程局部的类型擦除驱动器（`*mut ()` 上下文 + shim 函数）
  拿到 handler，由 `winit_run` 在事件循环存续期间注册。涉及的一切——事件
  循环、子类过程、定时器——都在同一个事件循环线程上，且定时器只在该线程
  阻塞于模态循环的消息泵内部时触发，那里栈上没有正在运行的帧，因此重入不
  可能与正在运行的帧交叠。
- `moonfield-winit` 新增仅 `cfg(windows)` 的 `windows-sys` 依赖（与 winit
  已在构建的 0.61 同线；本 crate 就是指定的 OS 绑定层）。

用同一程序化模态复现验证：修复后模态期间 `run_frame` 为 +42..+46/s（定时器
节奏减去帧耗时），而 `about_to_wait` 保持 +2/s——再叠加编辑器的
`MOONFIELD_EDITOR_SIM_RESIZE` 钩子强制每 tick 重建交换链，整个模态期间每帧
都在 recreate + present，零错误。

## Alternatives considered

- **依赖 `WM_PAINT` 的重入派发。** winit 在其 `WM_PAINT` 处理中同步派发
  `RedrawRequested`，但实测模态循环内 `WM_PAINT` 根本不会被递送，无可依赖。
- **winit 的 `with_msg_hook`。** 该钩子只能看到 winit 自己的消息泵取出的消
  息；模态循环在 `DefWindowProc` 内部自行泵消息，而
  `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` 是直接*发送*到窗口过程、从不入队——
  本修复需要的三条消息钩子一条也看不到。
- **用工作线程经 `EventLoopProxy` 驱动帧。** 用户事件由 winit 的消息泵处理，
  而泵恰恰处于阻塞中；模态循环退出前 proxy 唤不醒任何东西。
- **独立渲染线程。** 能把渲染与模态循环解耦，但会把 `App`/world 拆到多个线
  程——ECS 与整个帧循环按设计是单线程的。远比一个定时器侵入大。
- **常开的过期检测定时器（不做子类化）。** 在循环看起来停滞时就驱动一帧的
  定时器，无法区分模态阻塞与 Reactive 模式的空闲而不产生误触发；严格以
  `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` 为定时器开关是精确的且与模式无关。

## Consequences

- 在 Windows 上拖动或移动编辑器窗口时按定时器节奏持续渲染（约 40–60 fps，
  取决于帧耗时）；内容实时跟随窗口尺寸，因为每个定时器帧都走完整的
  update → recreate → present 路径。
- `WM_ENTERSIZEMOVE` 同样覆盖标题栏拖动与系统菜单；这些场景也会持续渲染，
  正是期望行为。
- 该机制在模态循环之外是惰性的：定时器只存在于 enter/exit 两条消息之间，
  正常模式的 pacing（`WinitSettings`，Continuous/Reactive）不受影响。
- 测量用的程序化模态复现（posted `WM_SYSCOMMAND(SC_SIZE)` + 键盘输入）用
  毕即移除；编辑器保留 `MOONFIELD_EDITOR_SIM_RESIZE` 钩子，它覆盖 recreate
  路径但无法复现模态阻塞——真实拖动仍是人工回归检查手段。

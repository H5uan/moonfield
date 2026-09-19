# Agent Note: Remove the modal-loop diagnostic scaffolding

Status: implemented

[English](2026-09-19-remove-modal-loop-diagnostics.md)

## Problem

[模态 sizing 循环调查](../bug-fix/2026-09-13-windows-modal-sizing-loop-frames.md)
把测量脚手架留在了源码里,每块都标注了测量后回退:`moonfield-winit` 中一个由
逐事件原子计数器和每秒报告线程组成的 `mod trace`(由 `MOONFIELD_WINIT_TRACE`
门控)、`moonfield-render-core` 中一个对应的 acquire/present 结果计数 `mod
trace`(由 `MOONFIELD_RENDER_TRACE` 门控),以及 `moonfield-winit` 中的
`sim_modal_resize`——一个用 SendInput 驱动真实鼠标拖拽并做 GDI 像素采集的回现
(由 `MOONFIELD_WINIT_SIM_MODAL_RESIZE` 门控)。这些脚手架同时导致 `cargo
clippy -- -D warnings` 失败:`sim_modal_resize` 里一个未使用的 `PostMessageW`
extern 和一处常量块大小的 `chunks_exact` 调用。

## Decision

删除全部三个代码块及所有调用点:`moonfield-winit` 事件处理路径
(`about_to_wait`、`RedrawRequested`、`Resized`、`run_frame`、`run_modal_frame`)
和 `WindowSurfaceData::acquire_image`/`present` 中的 `trace::bump` 调用、sizing
子类过程里的 `trace::set_modal` 调用、各环境变量门控,以及 `sim_modal_resize`
及其 Win32/GDI extern。它们所测量的机制——`sizing` 模块用定时器在模态循环中
驱动帧——是已交付的功能,保留不动。

## Alternatives considered

- **把诊断永久保留在环境变量门控之后。** 否决:这些计数器只为回答调查的问题
  而存在,答案已记录在 bug-fix 笔记中;留下的是死脚手架,代价是 clippy 违规和
  每次启用时一个永不退出的报告线程。
- **把 sim 回现保留在 cargo feature 之后以备未来回归。** 否决:该回现驱动真实
  OS 桌面(注入光标、抢夺前台),无法在 CI 运行。编辑器的
  `MOONFIELD_EDITOR_SIM_RESIZE` 钩子仍覆盖 swapchain 重建路径,而模态阻塞的手动
  回归检查仍是真实拖拽,如 bug-fix 笔记所记。

## Consequences

- 三个环境变量不复存在;没有任何文档引用它们,`docs/architecture.md` 不变。
- `cargo clippy -p moonfield-winit -p moonfield-render-core --all-targets --
  -D warnings` 通过;两个 crate 的测试不变且全绿。
- 模态拖拽期间的帧流动只能再通过 bug-fix 笔记中记录的测量或手动拖拽观察。

# Agent Note: FrameContext 持有帧命令缓冲

Status: implemented

[English](2026-09-06-frame-context-owns-frame-command-buffer.md)

## Problem

帧的命令缓冲原先逐窗口存放在 `WindowSurfaceData` 中。pass 系统从 `WindowSurfaces` 映射里捞出"任一进行中的窗口"来录制离屏 pass（`values_mut().next()` / `find_map(current_command_buffer)`），带来四个后果：没有窗口帧在进行时，离屏/视口 pass 静默地什么都不渲染；离屏工作提交在碰巧完成 acquire 的那个窗口上；帧槽有两个权威（逐窗口的 `FrameSequencer` 与 `FrameDrawArena` 的 `current`）；而 `device.begin_gpu_frame(slot)`——退休环的排空——每帧每窗口调用一次，一旦出现第二个窗口就会出错。

## Decision

渲染世界新增 `FrameContext` 资源（moonfield-render-core）持有帧本身：命令池、逐槽命令缓冲环、timeline 信号量，以及 `FrameSequencer` 状态机（帧号、槽位、进行中标志——状态机本身不变，只是逐窗口的图像跟踪移出了它）。`WindowSurfaceData` 只保留逐窗口状态：surface、swapchain、深度缓冲、二元 `image_available`/`render_finished` 信号量组，以及作为普通字段的已获取图像索引与重建标志。

- `acquire_window_frames` 在设备存在的每个 `Render` tick 都开始帧：timeline 等待、一次 `device.begin_gpu_frame(slot)`、命令缓冲 begin、描述符堆绑定。`WindowFrameDemand` 只门控 swapchain 的 acquire/present，不再门控帧是否存在。
- pass 系统从 `FrameContext` 读取槽位与命令缓冲；`FrameDrawArena::begin_frame(slot)` 从帧取槽位，槽位由此只有一个权威。离屏 pass 无条件录制；面向窗口的视图仍逐（进行中）surface 录制进同一条帧命令缓冲。
- `submit_window_frames` 只提交一次：等待每个已获取窗口的 `image_available[slot]`，signal 每个 `render_finished[slot]` 外加值为帧号的 timeline，然后 present 每个已获取窗口。没有任何窗口获取图像的帧只 signal timeline。
- `Device::submit_frame_timeline` 的等待与 signal 参数改为 `&[&Semaphore]` 切片。

## Alternatives considered

- **逐窗口持有（旧设计）。**否决：帧是设备级概念——一条命令缓冲、一根 timeline、一次退休排空——逐窗口持有让离屏渲染依赖不相干窗口的状态、把提交放在任意一个窗口上，还让退休排空随窗口数翻倍。
- **独立的离屏提交队列。**否决：第二条提交流会把 timeline 与退休环记账拆成两份，离屏工作与窗口 pass 之间（共享描述符堆、共享视图目标）的排序也得手工维护；每帧一条命令缓冲让执行顺序与录制顺序一致。

## Consequences

- 只要设备存在，离屏/视口 pass 每个 tick 都渲染，与窗口无关。
- `begin_gpu_frame` 每帧恰好执行一次；第二个窗口不再破坏退休环。
- 窗口最小化或失去 demand 时停止 acquire/present，帧循环与离屏渲染照常继续。
- 已呈现帧计数是帧级的（`FrameContext`）；编辑器的反馈通道从那里读取。

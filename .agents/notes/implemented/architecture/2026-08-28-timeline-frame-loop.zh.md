# Agent Note: Timeline 信号量帧循环

Status: implemented

[English](2026-08-28-timeline-frame-loop.md)

## Problem

每个窗口的帧循环用 fence 池节流：`MAX_FRAMES_IN_FLIGHT` 个逐槽 `Fence`，每次 acquire 都要显式 `wait` + `reset`，另有每组两套逐槽二元信号量。fence 是二态闩锁；帧节流与槽位复用需要的是单调计数器。

## Decision

`WindowSurfaceData` 用一根 timeline 信号量（`Semaphore::new_timeline(&device, 0)`）+ 从 1 起的 `frame_submitted` 计数器取代 in-flight fence 池——即参考项目（`no_gfx_api`）`frame_sem` 的形状：

- 帧 `n` 使用槽 `(n-1) % MAX_FRAMES_IN_FLIGHT`；在 acquire（以及在 `acquire_next_image` 重新 signal 该槽的二元 `image_available` 之前）先对 timeline 执行 `wait(frame_submitted - MAX_FRAMES_IN_FLIGHT)`。
- 提交路径是 `Device::submit_frame_timeline`，用 `vkQueueSubmit2`（`SubmitInfo2`）：在 color-attachment 阶段等待二元 acquire 信号量，signal 二元 present 信号量和值为当前帧号的 timeline，全程无 fence。timeline 值严格递增，因此整个循环中不存在 reset。
- `image_available` / `render_finished` 保持二元：`vkAcquireNextImageKHR` 与 `vkQueuePresentKHR` 都要求二元信号量，present 流程不动。

`Fence` 仍保留在 RHI 中（其它路径还在用），但帧循环不再使用。

## Alternatives considered

**为 present 把 timeline 桥接到单一二元信号量**（录制一条微型命令缓冲：先等 timeline 值再 signal 二元，参考项目即如此）。否决：每个窗口省两根二元信号量，代价是每帧多一次提交；而且 present 路径本来就命名一根二元信号量。

## Consequences

- 帧节流是单一计数器：第 `n` 帧开始前 `wait(n - MAX_FRAMES_IN_FLIGHT)`，提交时 signal `n`。每帧不再有 fence wait/reset。
- swapchain 重建（`device.wait_idle`）无需重置 timeline；计数器继续累加即可。
- 同一计数器就是上传路径竞技场回收所挂的帧信号（Phase 1 下一步）。
- 约束：若在槽的上一个周期完成前就 acquire，会让二元 `image_available` 二次 signal；timeline wait 始终先于 `acquire_next_image`。
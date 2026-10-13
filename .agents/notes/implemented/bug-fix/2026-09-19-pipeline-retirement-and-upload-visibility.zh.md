# Agent Note: Pipeline retirement and upload→frame memory visibility

Status: implemented

[English](2026-09-19-pipeline-retirement-and-upload-visibility.md)

## Problem

帧循环周边的三处 Vulkan 生命周期与同步缺陷：

1. `GraphicsPipeline::drop`/`ComputePipeline::drop` 立即调用
   `vkDestroyPipeline`，而最多 `MAX_FRAMES_IN_FLIGHT` 个在飞帧的命令缓冲
   仍可能绑定该 pipeline。着色器热重载即可触达：`Core3dPipelines::insert`
   在帧仍在飞时 drop 被替换的 pipeline。buffer、image、allocation 早已走
   设备的[退休环](../architecture/2026-09-03-device-retirement-ring.zh.md)，
   pipeline 没有。
2. `FrameUploader::end_frame` 用自己的 `queue_submit2` 提交批次，帧循环又
   把帧命令缓冲单独提交到同一队列。提交顺序只给两个批次排了执行先后，不
   构成内存依赖，因此写入 `Memory::Gpu` buffer 的 transfer 写（mesh 上传）
   没有任何 barrier 或信号量保证其对帧内 shader 读（device address 取顶点）
   可见。
3. `FrameUploader::drop` 从不等待在飞批次，析构时其 command pool 与
   command buffer 可能在批次仍在执行时被释放。

## Decision

- pipeline 走现有退休环：新增 `RetireAction::Pipeline` 携带裸 device 与
  pipeline 句柄，两个 pipeline 的 `Drop` 压环而非就地销毁。调用方零改动。
- 上传可见性搭乘 uploader 已有的 timeline 信号量。
  `FrameUploader::pending_signal` 暴露最近一次提交的值；
  `Device::submit_frame_timeline` 增加 `timeline_waits` 参数；
  `FrameContext::end_frame` flush uploader（flush 从
  `submit_window_frames` 移入帧自身的提交路径）并传入该等待点。
  `Device::submit_and_wait` 同样等待 uploader 的最近批次，覆盖测试与
  readback 路径。
- `FrameUploader::drop` 在其字段（command buffer，然后 pool）析构前等待
  最后提交的批次。

## Alternatives considered

- **在帧命令缓冲开头记录 `TRANSFER_WRITE`→`SHADER_READ` barrier。** 无资源
  的 barrier 只能覆盖所有 buffer，且对另行提交的上传批次仍不提供执行序。
  timeline 等待用一个 uploader 已有的机制同时给出顺序与可见性。
- **每个退休 pipeline 一把 fence。** 为帧槽环已经回答的生命周期问题成倍
  增加同步对象；环笔记对 buffer 的同一否决理由同样成立。
- **`FrameUploader::drop` 里 `vkDeviceWaitIdle`。** 正确但会停掉无关的在飞
  工作；timeline 等待恰好只覆盖 uploader 自己的提交。

## Consequences

- 渲染期间替换 pipeline（着色器热重载）安全：销毁在 drop 后
  `RETIRE_RING` 帧执行，最晚不超过设备析构。
- 每次帧提交至多增加一个 timeline 等待；无上传的帧等待的是已信号的值。
- `submit_frame_timeline` 增加了一个参数；render-core 帧循环是其唯一
  调用方。rhi 公共 API 仍不暴露后端类型（`verify_rhi_boundary.py` 通过）。
- `cargo test -p moonfield-rhi`（47 个测试，真实驱动，含
  `gpu_tests::upload_ring` 与 `headless_triangle`）不变通过。

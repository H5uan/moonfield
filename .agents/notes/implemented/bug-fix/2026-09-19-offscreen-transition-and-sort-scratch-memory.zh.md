# Agent Note: Offscreen transitions via the shared uploader; sort scratch is device-local

Status: implemented

[English](2026-09-19-offscreen-transition-and-sort-scratch-memory.md)

## Problem

代码审计发现的渲染栈两处性能缺陷：

1. 每次 `OffscreenTarget` 创建与 resize 都执行阻塞式布局转换：一次性
   command pool、command buffer 与 fence，然后在调用线程上
   `wait_for_fences(u64::MAX)`。调用方是 `PrepareViews` 集合里的
   `prepare_view_targets`，因此拖动编辑器视口时每帧都会停住图形队列。
2. `RadixSort` 的暂存 buffer（`tmp_keys`/`tmp_values`/`hist`/`offsets`）
   以 `Memory::Default`（CpuToGpu）分配：host-visible 内存每轮排序被
   scatter pass 读写多次，而 CPU 从不触碰。

## Decision

- `OffscreenTarget` 把新 image 的 `UNDEFINED` → `GENERAL` 转换记录进设备
  共享的 `FrameUploader`（`Device::uploader` →
  `FrameUploader::transition_image`，即为 storage image 建的路径），不再
  就地提交并等待。顺序保证：`FrameContext::end_frame` flush uploader，并
  以对 uploader 最近批次的 `ALL_COMMANDS` timeline 等待提交帧命令缓冲
  （`Device::submit_frame_timeline`），转换因此先于帧内任何命令完成——
  既包括渲染进该 target 的 pass，也包括 egui 对它的采样。帧循环之外的
  调用方（gpu 测试、readback 测试）在提交前自行 flush uploader；
  `Device::submit_and_wait` 已经等待 uploader 的最近批次。
- 四个暂存分配改为 `Memory::Gpu`（device-local）。只有调用方的
  `keys_in`/`values_in`/`keys_out`/`values_out` 保持 host-visible——host
  写输入、读输出。
- `Fence::raw` 失去唯一调用方，随之移除。

## Alternatives considered

- **把转换记录进帧命令缓冲。** `prepare_view_targets` 是 prepare 阶段的
  system，没有 `RenderContext` door，帧命令缓冲得一路传下去；uploader
  路径不需要任何穿线，且本就合批本帧的其他转换与上传。
- **在 `OffscreenTarget::create`/`resize` 内部 flush uploader。** 那里的
  `end_frame` 只提交不阻塞，测试调用方可以不动；但它会把帧的上传批次
  按 target 拆开，并让创建类 API 把提交 GPU 工作变成副作用。在帧边界
  （以及测试中）显式 flush，与 `Texture::bindless` 调用方已有的用法一致。
- **sort 暂存维持 `Memory::Default`。** 没有任何收益——没有代码路径映射
  这些 buffer——还把 host-visible 内存带宽花在纯 GPU 暂存上。

## Consequences

- 视口创建与 resize 的代价降为在已在录制的批次里多记一条 barrier；
  渲染线程不再在此处等待队列。
- `OffscreenTarget` 的 headless 与测试调用方多一步：在自己的提交前
  flush `Device::uploader()`（编辑器的 egui headless 测试本就如此；
  rhi 的 gpu 测试与 core-3d pass 测试补上了 flush）。
- radix sort 的确定性验收测试在 device-local 暂存下不变通过。

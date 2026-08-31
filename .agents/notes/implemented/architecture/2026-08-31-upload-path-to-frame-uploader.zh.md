# Agent Note: 上传路径接入帧上传器

Status: implemented

[English](2026-08-31-upload-path-to-frame-uploader.md)

## Problem

`Buffer::upload`（GpuOnly）与 `Texture::upload` 每次调用都新建一个 staging buffer、一个 command pool 和一根一次性命令缓冲，然后阻塞在 `queue_wait_idle` 上——正是 bump arena 要消除的逐调用创建/销毁开销。帧上传器需要服务它们，但它借用 `&Device`，无法进入 ECS 资源（`EguiTextures`）或挂在设备本体上。

## Decision

- **去生命周期的上传器。** `GpuAllocation::from_resources` 用自持的 `ash::Device` + `Arc<Allocator>` 建块；`GpuBumpAllocator` 与 `FrameUploader` 改存这些 Owned 资源而非 `&Device`，从而兼容 `'static`（`HostPtr` 亦补 `Sync`，沿用既有的单写者契约）。所有构造函数保持 `&Device` 签名。
- **设备托管的共享上传器。** `Device` 懒建一个 `OnceLock<Arc<Mutex<FrameUploader>>>`（字段声明在 `allocator` 之前，先 drop，保证在分配器存活时归还各块）。`Buffer::upload` 的 GpuOnly 分支经 `uploader.upload_and_wait` 走——调用签名不变，调用方零改动。
- **纹理上传委托。** `Texture::upload` 收 `&mut FrameUploader`，经 `FrameUploader::upload_image` 录制——原 `texture.rs` 里的布局转换 barrier 归其所有。`begin_frame`/`end_frame` 幂等：空帧不提交。
- **每帧一次 flush。** `EguiTextures` 自持一个 `FrameUploader`（`upload_pool` 字段删除）；`prepare_egui_frame` 在帧尾调一次 `flush_uploads`，所有纹理增量一次提交。
- **实例扩展不是设备扩展。** `VK_KHR_surface` 不进设备启用列表：NVIDIA 驱动会以 `ERROR_EXTENSION_NOT_PRESENT` 拒绝实例扩展出现在该列表；为换取真实可跑的驱动，接受 validation VUID 01387 的提示噪音。

## Alternatives considered

**显式逐调用点传 uploader。** 否决：几十个调用点（mesh 加载、测试）都要改；设备托管的共享上传器保持 `Buffer::upload(device, data)` 原样。

**编辑器每帧新建一个 `FrameUploader`。** 否决：每帧 8 MiB arena 分配与对象churn，无收益。

## Consequences

- 任何调用都不再有逐次 staging 创建：上传从 arena 切内存，以每帧一次提交（纹理）或一次提交+等待（载入期 buffer）发出。
- 设备托管的上传器先于分配器 drop（字段顺序），块归还发生在设备与分配器仍有效时。
- `upload_ring.rs` 跑的正是编辑器运行的同一套代码；既有的 buffer/tests 路径即共享上传器路径的回归套件。

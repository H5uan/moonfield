# Agent Note: 帧级上传器，逐槽竞技场

Status: implemented

[English](2026-08-28-frame-uploader.md)

## Problem

向 `GpuOnly`（device-local）buffer 上传意味着**每次调用**都新建 staging buffer、command pool 和一次性命令缓冲，然后阻塞在 `queue_wait_idle` 上。staging 是帧级生命周期的临时数据——正是 bump arena 建模的对象；阻塞是"每次上传"粒度而非"每帧"粒度。

## Decision

`moonfield-rhi` 的 `vulkan/upload.rs` 拥有 `FrameUploader<'a>`——参考项目的逐帧 arena 形态接到 timeline 上：

- `UPLOAD_FRAME_RING` 个槽位，每个槽持有一个 `GpuBumpAllocator` **和它自己的命令缓冲**。`begin_frame` 在 timeline 上 `wait(next_frame - RING)`，然后 `free_all` 该槽的 arena 并重录该槽的缓冲；`upload` 追加拷贝；`end_frame` 提交一次，并以其帧号 signal timeline。
- 槽的回收信号**恰好就是**该槽 arena 与命令缓冲可以安全复用的信号——**arena 与命令缓冲必须共享同一复用周期**（`wait(n - RING)`）。重录仍在执行的 `ONE_TIME_SUBMIT` 缓冲是未定义行为，因此单一共享缓冲无法服务会超前一帧的 ring。
- `upload` 只接受 `GpuOnly` 目标，经 `BumpAlloc` 做 staging（`cpu` 用于 memcpy，`src`/`src_offset` 用于拷贝命令）。host-visible 目标由调用方直接写入，这里予以拒绝。
- `upload_and_wait` 是一次性的载入路径：begin、upload、end、wait。

`VK_KHR_surface` 加入 `REQUIRED_DEVICE_EXTENSIONS`：它是 `VK_KHR_swapchain` 的必需扩展，validation（VUID 01387）拒绝在未启用它时启用 swapchain；该错误还级联出大量虚假的 feature/allocator VUID。

## Alternatives considered

**所有槽共用一根命令缓冲。** 实践中否决：ring 让第 `n` 帧在第 `n-1` 帧仍在执行时就能开始，仅当槽内资源与节拍匹配；单根缓冲被迫 `wait(n - 1)`，把管线串行化。

## Consequences

- 一帧内多次上传 = 一次提交；staging 与命令对象在 uploader 创建时建一次，而非每次上传建一次。
- `FrameUploader` 的字段顺序 matters：`cb`（drop 时调用 `vkFreeCommandBuffers`）声明在 `pool` 之前（其 drop 销毁池并释放所有存活的缓冲）——结构体字段按声明序 drop，与局部变量相反。
- 设备创建通过 validation：swapchain 的扩展依赖显式声明。
- 消费方（`Buffer::upload`、`Texture::upload`、编辑器的 egui 纹理路径）下一步迁移到此 uploader；headless 测试自行驱动完整帧循环（`upload_ring.rs`）。
# Agent Note: rhi storage image

Status: implemented

[English](2026-09-09-rhi-storage-image.md)

## Problem

Gaussian splatting 的合成 pass 需要一张 compute 内核写、光栅 pass 采样的中间图像（`RGBA16F`、`SAMPLED|STORAGE` usage）。rhi 没有可存储写的图像构造器、没有浮点颜色格式，且整条链在 descriptor-heap 路径上从未验证过：`STORAGE_IMAGE` 堆描述符、Slang 对堆寻址 `RWTexture2D` 的代码生成、T1000 的 RGBA16F optimal-tiling 存储写，都没跑过。

## Decision

- `Texture::storage_image(device, uploader, width, height, format)`——重构决策豁免的那一个 rhi 加法构造器——创建 `SAMPLED|STORAGE` 图像，**一个视图两个 descriptor-heap 槽**：compute `RWTexture2D` 写入的 storage-image 槽（`Texture::storage_handle`）与着色器读取的 sampled-image 槽（`Texture::handle`）。`Drop` 双槽退休。
- 图像的 `UNDEFINED -> GENERAL` 初始化经 `FrameUploader::transition_image`（crate 内部）——一次无拷贝的图像屏障，统一布局保证认可的例外；调用者在首次 dispatch 前以 `end_frame` 提交。
- `Format::R16G16B16A16Sfloat` 进入格式枚举。
- 设备创建请求 `shaderStorageImageWriteWithoutFormat` 与 `shaderStorageImageReadWithoutFormat`：堆寻址的 `RWTexture2D` 没有可注格式的声明处，Slang 发出 `Format=Unknown` 的 `OpTypeImage`。
- probe（`gpu_tests::storage_image`）验证全链路：compute 内核经 storage 槽按 texel 存 `(x, y, 0.5, 1)`，`barrier(COMPUTE, COMPUTE, Memory)` 定序，第二个 dispatch 经 sampled 槽 `Texture2D.Load` 同一图像进回读缓冲——在 T1000 上逐 texel 验证半精度精确值。

probe 钉死的两个静默失败坑，此后是所有堆着色器作者的常备知识：

- **Slang 在未向 `compile_*_with_capabilities` 传 `spvDescriptorHeapEXT` capability 时，把 `ResourceDescriptorHeap[…]` 访问编译成死代码——不报错。** 两个 probe 内核都需要它。
- **dispatch 前必须把堆绑定到命令缓冲（`heap.cmd_bind(&cmd)`）**，否则堆访问静默读零。

## Alternatives considered

- **零填充上传做初始化。** 落选：只为一次布局转换却强加 `TRANSFER_DST` usage 与全尺寸暂存拷贝；`transition_image` 表里如一。
- **构造时的格式能力检查。** 落选：`Device` 不持有 `Instance`，构造器内无法调 `vkGetPhysicalDeviceFormatProperties`；probe 即验证，GS 路径真正接入时再捎带 fail-fast 检查。
- **按类型给资源堆定尺寸。** 实测排除：T1000 上 `vkGetPhysicalDeviceDescriptorSizeEXT` 报告 `SAMPLED_IMAGE` 与 `STORAGE_IMAGE` 均为 32 字节（buffer 16、uniform 8），堆现有 image stride 覆盖 storage 描述符。

## Consequences

- GS blend 内核的中间图像可以构造了：一次调用得到两个句柄，合成 pass 走现有采样路径。
- 两个 `WithoutFormat` 特性被无条件请求（T1000 支持；没有堆能力 GPU 的 CI 本来就跳过设备创建）。
- `ImageDescriptorKind`（sampled/storage）是 `TextureSlotDesc` 的 crate 内部细节——堆的公开表面不变，`verify_rhi_boundary.py` 通过。

# Agent Note: core 3d pass bindless root data

Status: implemented

[English](2026-09-01-core-3d-bindless-root-data.md)

## Problem

Core3D 不透明 pass 一直把整个逐 draw 负载——mvp + color 共 80 字节的块——通过 `vkCmdPushConstants` 内联推送。这让图形管线停留在保留式 push-constant 模型上，而 RHI 的 bindless 2.0 基础（描述符堆、`GpuBumpAllocator`、根指针）早已在 compute 路径上得到验证：Slang 把 shader 入口的 `Ptr<T>` 参数降级成一个持有设备地址的 push-constant 块，因此 draw 数据可以住在 GPU 内存里，只有指针穿过命令缓冲。

## Decision

- `ScenePushConstants`（mvp + color，80 B）更名为 `DrawData`，逐 draw 存入新的 render-world 资源 `FrameDrawArena`。
- `FrameDrawArena` 镜像 `FrameUploader` 的环形结构：`MAX_FRAMES_IN_FLIGHT` 个 `GpuBumpAllocator`；`begin_frame(slot)` 在 `acquire_window_frames` 的 timeline wait 证毕该槽上一轮 GPU 工作完成后对它 `free_all`；`alloc_draw_data` 从当前槽刻画一块 `DrawData`。内部 `Mutex` 沿用 `DescriptorHeap` 的先例，使得只拿到 `&World` 的 draw 函数也能分配。
- 管线按 descriptor-heap 模式创建——null layout，无任何 push 范围——根是单个 `GpuPtr`（`ROOT_POINTER_SIZE`，8 B），经 push data（`push_data`）送达；`DrawMesh` 经 bump host 指针写入 `DrawData`，并把设备地址作为根推送。顶点/索引缓冲保留经典绑定——本里程碑只改根数据。
- Shader 以入口参数声明 `Ptr<DrawData> root`，读 `root[0].mvp` / `root[0].color`；矩阵仍为 `column_major` + `to_cols_array()`，因此到达 GPU 的字节不变。
- `Stage::VERTEX` / `Stage::FRAGMENT` 加入 `bindless.rs`，使图形阶段的 bindless barrier 能表达同一指针模型。

## Alternatives considered

- 用字面 `push_data`（descriptor heap 的 root bank）取代 push constant 里的根指针：当时 `vkCmdPushDataEXT` 的 GPU 侧消费端尚未接线，且 Slang 的入口 `Ptr` 降级仅对 push constant 有验证，故先落地"指针经 push constant"的形式。`command_push_data` 验证 GPU 侧消费后，[push-data-only 清理](../simplification/2026-09-02-descriptor-heap-push-data-only.md) 把所有管线迁到 push data 并删除了 retained 模型。
- 在 push constants 里保留 80 字节负载内联：pass 停在保留式模型上，bindless 管线毫无变化。

## Consequences

- 图形 pass 已呈 bindless 形态：逐 draw 根数据是单个 GPU 指针，负载住在可复用的 GPU 内存里，后续纹理/材质直接并入同一根结构。
- 根数据从 80 B 的内联推送缩成一个设备地址，经 descriptor-heap 的 push-data bank 流动——管线中不再有任何 set layout 或 push-constant 范围。
- draw arena 的帧节拍搭在窗口帧 timeline 上；单窗口槽位假设已为多窗口未来记录在案。
- 测试：`test_opaque_pass_draws_mesh` 与 `test_opaque_pass_depth_occludes` 未经改动继续通过——它们现已走过指针路径；headless 测试因无窗口帧循环而手动驱动 arena 槽 0。
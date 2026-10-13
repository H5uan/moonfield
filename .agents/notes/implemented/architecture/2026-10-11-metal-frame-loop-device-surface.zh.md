# Agent Note: Metal frame-loop device surface

Status: implemented

**Superseded:** Metal 后端已移除——见
[the Vulkan pivot note](2026-10-13-rhi-vulkan-pivot.zh.md)。

[English](2026-10-11-metal-frame-loop-device-surface.md)

## Problem

[Metal 4 提案](../../rejected/architecture/2026-10-09-metal-4-backend.zh.md)
的 phase 3 交付了 `CAMetalLayer` 交换链与平台选择的 `RenderDevice`，但
Selene 窗口帧循环无法对 Metal 后端编译：它拼写的设备面后端并不携带 ——
`begin_gpu_frame`、`wait_idle`、`supports_present`、uploader 的
`end_frame`/`abort_frame`、heap 的 `cmd_bind`、`Swapchain::take_aborted`、
`GraphicsPipeline::id`、`OffscreenTarget`、`DepthBuffer` —— 且引擎层把
GPU 句柄存进 ECS 资源，需要 `Send + Sync`（objc2 对象默认两者皆无）。

## Decision

Metal 后端携带帧循环的设备面，映射到 Metal 的模型：

- **帧步调与退休。** `begin_gpu_frame(slot)` 排空该槽位的退休队列并标记
  为推送目标；`flush_retirements` 全量排空。已提交命令缓冲可能仍经原始
  GPU 地址（argument-table 槽位、root-blob 环）引用的资源，其最终释放
  延迟进环：Metal 命令缓冲不保留按地址绑定的资源。`Buffer` 的 `Drop`
  推送延迟释放；encoder 绑定的附件与管线由 Metal 保留，无需延迟。
  `wait_idle` 在队列已提交工作之后入队一个时间线信号并等待它。
- **uploader。** `FrameUploader::upload_alloc` 把字节暂存进按需增长的
  共享存储缓冲，并在经典 `MTLCommandBuffer` 上记录 blit 拷贝 ——
  `objc2-metal` 0.3.2 的 Metal 4 命令缓冲不带 blit encoder，uploader
  因此自持经典命令队列。`end_frame` 提交批次并等待完成，随后提交的帧
  得以观察这些拷贝（同步等待以 CPU-GPU 重叠换取免跨队列事件的顺序性；
  若性能分析显示代价，以事件排序路径替换）。`abort_frame` 不提交地丢弃
  批次。host 可见分配无需 uploader：共享存储使其写入即直接。
- **目标。** `OffscreenTarget`（颜色 + 可选 `D32Sfloat` 深度，
  视图/格式/尺寸/resize/读回）与 `DepthBuffer`（窗口尺寸的 `D32Sfloat`
  深度）由渲染目标 `Texture` 构建。其 bindless 句柄留在 phase 2 —— 已
  编译层中没有消费方拼写它们。
- **bindless 绑定点。** `DescriptorHeap::cmd_bind` 不记录任何东西：本
  后端上逐命令缓冲的 argument table 就是资源通道，没有设备级全局 heap
  可绑定。`bindless_textures` 能力在 `DescriptorHandle` 通道（phase 2）
  落地前保持 `false`。
- **零散对等项。** `Instance::supports_present` 恒为真（每个 Metal 设备
  都能呈现到 `CAMetalLayer`）；`Swapchain::take_aborted` 恒为假（阻塞
  式获取没有"已信号但未消费"状态）；`GraphicsPipeline::id` 是管线对象
  地址（绑定跟踪的去重键）。
- **所有权。** 公共句柄在 RHI 的主线程所有权规则下携带 `Send + Sync`
  （录制、提交与资源使用由所属帧循环外部同步 —— 与 Vulkan 后端依赖的
  同一契约）。这些 impl 集中在一个带文档的模块；`CommandBuffer` 此前
  已携带同一对。

## Alternatives considered

- **经 `msg_send!` 在 Metal 4 命令缓冲上取 blit encoder。** 弃用原因：
  该选择器未对照 Metal 4 头文件验证，而经典队列在 `objc2-metal` 中
  完整可用 —— 即提案覆盖缺口的回退路径。
- **仅靠 Metal 对象生命周期实现延迟退休。** 弃用原因：按地址绑定的资源
  恰是命令缓冲不保留的那类；没有环，飞行途中丢弃每帧缓冲会留下悬空的
  GPU 地址。
- **现在就做事件排序的非阻塞 uploader。** 弃用原因：跨队列事件排序只为
  有上传的帧买回 CPU-GPU 重叠；在性能分析显示代价之前，同步等待是简单
  正确的基线。

## Consequences

- `moonfield-render-core`、`moonfield-render-feature`、`moonfield-ml`
  对 Metal 后端编译通过，测试在 Apple Silicon 上全部通过 —— 包括帧循环
  的 abort GPU 测试（acquire → abort → 重试 → 经完整设备面提交）与 ml
  trainer 的 `gaussian_fit_converges_through_trainer`（提案 phase 4 的
  ml autodiff→metallib 验证）。
- editor 在 Metal 路径上的剩余缺口是 bindless 通道
  （`TextureHandle`/`SamplerHandle`、`sampler_for`、`Texture::bindless`
  与 `Texture::upload`、离屏句柄）加 egui 的 Metal 移植 —— Metal 提案的
  phase 2 与 phase 5。
- 引擎层中经 `Texture::upload`/`bindless` 的上传消费方（editor 的图集
  纹理）随该通道落地；`upload_alloc` 服务 render-feature 今天拼写的
  GPU 私有静态几何路径。

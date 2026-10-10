# Agent Note: Metal 4 backend skeleton

Status: implemented

[English](2026-10-09-metal-4-skeleton.md)

## Problem

macOS 没有 GPU 路径：Vulkan 后端的设备表无法被 MoltenVK 满足，Apple silicon
上的本地开发完全没有渲染器可用。[Metal 4 后端提案](../../proposed/architecture/2026-10-09-metal-4-backend.zh.md)
把后端分了阶段；第一阶段是骨架——在真实硬件上证明 Metal 4 面：设备门控、
队列与常驻、带 argument table 的命令录制、以及一次像素校验的绘制。

## Decision

`moonfield-rhi-metal`（位于 `crates/moonfield-rhi/metal/`，基于
`objc2-metal` 的 `MTL4*` 类型）交付骨架：

- `Instance::new_headless` 获取系统默认设备并执行平台门控 ——
  `supportsFamily(MTLGPUFamily::Metal4)`，否则给出明确错误。
- `Device` 持有一个 `MTL4CommandQueue`、设备级 `MTLResidencySet`
  （每个 buffer/texture 在创建时自注册；该集合与设备同生命周期）、一个
  `MTL4CommandAllocator` 和 `MTLSharedEvent` timeline。
  `submit_and_wait` 是 `endCommandBuffer` → 队列 `commit_count` → 队列
  `signalEvent_value` → CPU `waitUntilSignaledValue`：队列级事件信号就是
  fence，因为 `MTL4CommandBuffer` 不继承经典的 `waitUntilCompleted` API。
- `CommandBuffer` 在 `beginCommandBufferWithAllocator` 上录制，持有自己的
  `MTL4ArgumentTable`（8 个 buffer 槽位）。buffer 绑定走
  `setAddress_atIndex` —— shader 侧看到 `[[buffer(index)]]`；Metal 4
  encoder 上没有 `setVertexBuffer`，argument table 就是绑定模型。render
  pass 消费 core 词汇（`RenderAttachment`/`RenderPassDesc`），取第一个
  color attachment。
- `Memory` 是共享存储的 `MTLBuffer`（统一内存：CPU 经 `contents` 写、GPU
  地址经 `gpuAddress`）；`Texture` 是渲染目标 + shader 可读的 2D 图像，
  带 CPU 读回；`GraphicsPipeline` 是顶点+片元、单 color format。
- `ShaderModule` 既可加载运行时编译的 MSL（`from_msl`），也可加载共享
  Slang 编译器产出的 `.metallib` 归档（`from_metallib`、`from_compiled`）；
  `GraphicsPipeline::from_modules` 用 Slang 按入口点生成的库构建管线。
  `slang_metallib` GPU 测试把 Vulkan 后端的顶点拉取 shader 形态
  （`SV_VertexID` + `Ptr<T>`）以 `ShaderTarget::MetalLib` 编译并经
  argument-table 槽位 0 渲染 —— 一份 Slang 源服务两个后端。
- 门面增加 `metal` feature，并以 `compile_error!` 拒绝两个后端 feature
  同时启用的构建；metal 再导出清单是 Vulkan 表面已实现的子集。
  `verify_rhi_boundary.py` 与其余源码树一起扫描 `metal/src`。

GPU 测试 `metal/src/gpu_tests/offscreen_triangle.rs` 渲染一个顶点拉取的
三角形（位置数据经 argument-table 槽位 0）到 64×64 目标，提交后在
timeline 上等待，并校验像素 —— 在 Metal 4 硬件上通过，其他环境带原因
跳过。

## Alternatives considered

**每命令缓冲的 residency（每个 buffer `useResidencySet` + 逐 pass 注册）**
能把常驻范围限定到 pass 实际触碰的资源。它落败于引擎的全局 bindless heap
模型：资源是设备生命周期的，逐 pass 记账是纯开销；创建时注册的设备级
集合与之一致。

**经典 `MTLCommandBuffer` API**（`waitUntilCompleted`、
`addCompletedHandler`）无法支撑 `MTL4CommandBuffer` —— Metal 4 命令缓冲是
独立协议，没有这些方法；队列级 `MTLSharedEvent` 信号/等待就是它的同步
模型。

**冒烟测试用离线编译的 `.metallib`** 可以提前搭好 Phase 2 的 Slang 路径，
但构建期需要工具链。运行时 MSL 编译让骨架自包含；`.metallib` 随 Slang
目标工作落地。

## Consequences

Apple silicon 有了本地开发和 CI 的 GPU 路径：后端可编译、冒烟测试在任何
Metal 4 设备上运行。下游 crate 不变 —— 所有平台仍然选择 `vulkan`；`metal`
feature 只再导出已实现的子集，消费方拿不到未实现的名字。骨架的 render
pass 只取单个 color attachment、忽略 depth attachment；
`AttachmentLayout` 是角色标记，没有 Metal 图像布局语义。剩余阶段
（`CAMetalLayer` swapchain、能力门控特性、egui 移植）仍由提案笔记持有。

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
- 展示：`Surface` 持有 `CAMetalLayer` —— 供离屏测试/渲染的独立可读回层
  （`new_layer`），或经 raw-window-handle 挂到窗口视图（`from_window`，
  AppKit `NSView`）。`Swapchain` 获取 layer 的下一个 drawable（阻塞；
  序号恒为 0），以 `TextureView` 暴露，并在队列 `signalDrawable` 把展示
  排到已提交工作之后后 present。`Semaphore` 对齐 Vulkan 表面形状（Metal
  的顺序就是队列提交序 —— 模块文档记录了该模型），`RenderDevice` 为引擎
  层插件配对门控后的 instance 与 device。`swapchain` GPU 测试从离屏 layer
  获取 drawable、经 swapchain 视图渲染、校验 BGRA 像素并在 Metal 4 硬件
  上 present。
- 计算与暂存：`CommandBuffer` 录制 compute pass
  （`begin_compute`/`set_compute_pipeline`/`dispatch`/`end_compute`），
  threads-per-threadgroup 取自管线；`ComputePipeline` 从 `.metallib`（或
  MSL）入口构建。`GpuBumpAllocator`/`BumpAlloc`/`HostPtr` 对齐 Vulkan
  表面 —— 统一内存下 bump 就是指向同一共享 `MTLBuffer` 的 CPU/GPU 指针
  对，无需上传通道。
- Metal 绑定模型（由 `compute` GPU 测试实测得出）：
  - Graphics 入口（顶点拉取）把 `Ptr<T>` root 直接绑在 argument-table
    槽位 0（`slang_metallib` 直绑顶点数组并渲染）。
  - Compute 入口把 root 参数打包成槽位 0 的 `EntryPointParams` 结构体 ——
    即 Vulkan `RootBinder` 构造的 root-blob 布局；绑定 blob（指针字段携带
    GPU 地址）就是 `push_data` 的 Metal 对应物。
  - `main` 入口在产出的库中被改名为 `main_0`；其他名字保留。
  `autodiff_fwd_numeric` GPU 测试把 `[Differentiable]` 代码以 `fwd_diff`
  编译到 metallib、经 root blob 派发并对照解析导数校验前向导数 —— 即
  ml 路径在该后端的编译目标。
- 引擎层录制表面：`begin_rendering`/`end_rendering`、`draw`、动态
  `set_viewport`（Y 翻转与 reverse-Z 的适配就在这里）、`set_scissor`、
  `set_cull_state`、`set_depth_state`（depth attachment 落地前记录意图）、
  `set_blend_state`（混合管线随 egui 移植带来）、
  `bind_graphics_pipeline`/`bind_pipeline`、`barrier`（`Stage`/`Access`
  镜像 Vulkan 词汇；Metal 4 的 encoder barrier 是 stage 到 stage）、
  `set_bindless_root` 与 `push_data`。
- `push_data` 快照：encoder 在首次使用时固化其 argument table —— 对同一
  table 重新 `setAddress`、甚至用新 table `setArgumentTable` 都无法在
  draw 之间重绑（实测）。因此每次 push 把 root blob 写进私有 ring、绑到
  新建的 argument table，且当活动 encoder 已编码过命令时重建 encoder：
  compute encoder 重绑管线；render encoder 以 `Load` 重建 pass（保留已
  渲染内容）并重放 viewport 与管线。
  `push_data_snapshots_between_dispatches` GPU 测试钉住该语义：两次
  push、两次 dispatch，各读各的值。

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
`AttachmentLayout` 是角色标记，没有 Metal 图像布局语义。到编辑器真正跑起来
的剩余差距是引擎层的资源机制（Metal 侧的 uploader、bump 分配器、bindless
堆）与 egui 移植；两者都由提案笔记跟踪。

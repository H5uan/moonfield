# Agent Note: Metal 4 backend

Status: rejected — abandoned by the [RHI Vulkan pivot](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.md)

**Shelved:** Metal 4 后端因 Vulkan pivot 而移除——见
[the Vulkan pivot note](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.zh.md)。

[English](2026-10-09-metal-4-backend.md)

## Problem

支持的目标平台是 Windows 和 Linux。macOS 需要一个原生后端：MoltenVK 无法
暴露 `Device::new` 所要求的设备表（`VK_EXT_descriptor_heap`、mesh shading、
ray tracing），因此 Vulkan 路线在 macOS 上走不通 —— Apple silicon 上的本地
开发目前完全没有 GPU 路径。平台要求：macOS 默认使用最新的 Metal 4
（`MTLGPUFamily.Metal4`，macOS 26 SDK），设备不达标时给出清晰的错误。约束：
下游 crate 消费的是 RHI 的具体类型，因此新后端必须匹配 Vulkan 后端的公共
名称与形状，而不是引入抽象层。

**搁置（2026-10-11）：** bindless 通道（phase 2）与 egui 移植（phase 5）暂停。
引擎保留 push data + descriptor heap 资源模型 —— DX12 与 Vulkan 都支持的方向
—— 而把 root 结构迁移到 Metal 要求的 `DescriptorHandle` 形状（argument-table
槽位、无 root-blob 字节；见下方实测）会让 binder 与引擎的 root 推送按后端
分裂。备选的 `ConstantBuffer<T>` 通道（实测：Metal 把其 handle 字段落为绑定
缓冲内 CPU 可写的内联 `gpuResourceID`/`gpuAddress` u64 —— 即 Zenith.NET 走的
形状）能统一写入路径，但要放弃逐 draw root 的 push data，是相反的取舍。
Phase 1–4 已落地的成果保留：Metal 后端可编译，`render-core`/
`render-feature`/`ml` 在 Apple Silicon 上编译并通过（见
[帧循环笔记](../../implemented/architecture/2026-10-11-metal-frame-loop-device-surface.zh.md)），
editor 的 Vulkan 路径不变。在此之前 editor 在 macOS 上无法构建 —— 重启的触发
条件是 Metal 出现承载 push data 模型语义的表面，或决定把 root 投递改为绑定
常量缓冲。

## Proposal

落地 `metal/` 子 crate（`moonfield-rhi-metal`，与 `moonfield-rhi-vulkan`
对称），基于 `objc2` + `objc2-metal`（它携带 `MTL4*` 类型）实现同一份精心
维护的公共类型清单。门面增加 `metal` feature，并用 `compile_error!` 拒绝
两个后端 feature 同时启用的构建；各消费 manifest 从 `vulkan` 切换为按目标
平台拆分（非 Apple 平台保持 `vulkan`，Apple 平台取 `metal`）。

API 映射，逐后端给出：

| Vulkan 概念 | Metal 4 概念 |
|---|---|
| `VK_EXT_descriptor_heap` 堆 + `cmd_bind_*_heap` | `MTL4ArgumentTable`（每命令缓冲一个；`SetAddress(buffer.gpuAddress + offset, stride, slot)`，对 render encoder 以 vertex/fragment/object/mesh 阶段绑定、对 compute encoder 绑定） |
| 堆/资源常驻 | 设备级 `MTLResidencySet`，加入每个 `MTL4CommandQueue`；加锁下 register/unregister allocation |
| `Fence`/`Semaphore` + retire ring | `MTLSharedEvent`（单调 signaled value 驱动 retire ring） |
| `Stage`/`Access` barrier | Metal 4 `BarrierAfterEncoderStages`（stage 到 stage）；纹理 layout 转换是空操作 |
| `TimestampQueryPool` | `MTLCounterSampleBuffer` + `Device.QueryTimestampFrequency` |
| Indirect draw/dispatch、GPU memcpy（`device_address_commands`） | 基于 device address 的 dispatch/copy（`MTL4CommandBuffer`/encoder） |
| `Surface`/`Swapchain`（ash-window） | `CAMetalLayer`（经 raw-window-handle）、`NextDrawable`，`SignalDrawable` 之后再 present |
| gpu-allocator `vulkan` 切片 | gpu-allocator `metal` 切片（同一 crate、同一版本），以 `MTLBuffer.gpuAddress` 支撑 `GpuAllocation`/`GpuPtr`/`Memory` |

着色器从同一份 Slang 源编译：`slangc -target metallib -capability
metallib_latest -Xmetal -std=metal4.0` 直接产出 `.metallib`（不经过 MSL
中转），用 `Device.MakeLibrary` 加载。`Compiler` 增加目标参数；
`Reflection`→绑定的映射保持各后端私有（Vulkan 的 `RootBinder` 往 push blob
里写指针字节；Metal 侧把 reflection 映射到 argument table 槽位）。能力门控
在平台有差异处取代硬性扩展表：mesh shading 是 `MTLGPUFamily.Apple7`+，ray
tracing 运行时上报，设备缺失的能力降级而不是让设备创建失败。引擎级 Y-up
reverse-Z 裁剪空间约定在 Metal 边界内适配（Metal NDC 是 z∈[0,1]），永远不
进 scene 代码。editor 的 egui 后端需要 `egui_vk.rs` 的 Metal 兄弟实现，
macOS 才能跑起 editor。

排序，每个阶段可独立交付：

1. `moonfield-rhi-metal` 骨架：MTL4 设备门控（`SupportsFamily(Metal4)`，
   否则报错）、`MTL4CommandQueue` + residency set、`MTL4CommandBuffer` +
   argument table、`MTLSharedEvent` timeline、barrier、copy；
   `metal/src/gpu_tests/` 下的离屏三角形冒烟测试。
2. `Compiler` 的 Slang `metallib` 目标、`ShaderModule::from_metallib`、
   reflection→argument-table 槽位。
3. `CAMetalLayer` swapchain + 按平台选择的 `RenderDevice`，点亮 macOS 上的
   Selene 窗口帧循环。
4. mesh shading pipeline、时间戳、uploader/bump 对应物；ml 的
   autodiff→`metallib` 单独验证。
5. editor 的 egui Metal 后端。

## Alternatives considered

**macOS 上用 MoltenVK** 可以原样复用 Vulkan 后端。它落败是因为 RHI 的设备表
要求 MoltenVK 不暴露的 `VK_EXT_descriptor_heap` 与 mesh/ray-tracing 扩展 ——
后端将不得不降级到残缺的特性集，而那正是原生后端用能力门控避免的处境。

**在 Metal 工作之前先做运行时 trait 层** 能让两个后端运行时可选、后端选择
成为库层关注点。它落败的原因与
[子 crate 笔记](../../implemented/architecture/2026-10-09-rhi-backend-subcrates.zh.md)
记录的相同：它为一个 editor-only 构建图不需要的选择能力改写所有下游签名；
互斥 feature 下的同名再导出已经交付了平台选择而不带来这些改动。

**以 Metal 3 为基线** 能覆盖更多机器（任何搭载 Apple silicon 的 macOS）。它
落败于"macOS 默认最新 Metal 4"的要求：argument table、residency set、新
barrier 模型正是与 RHI 的 bindless/同步词汇对应的 Metal 4 面；瞄准 Metal 3
意味着用 argument buffer 和手工追踪去近似这些 —— 同样多的工作量，却是另一个
后端。

## Acceptance criteria

- 在 M 系列 Mac 上：`cargo run` 以 `metal` feature 构建 `moonfield-editor`
  （依赖图中没有 `ash`），通过 Selene 帧循环渲染 editor viewport。
- 不带 `MTLGPUFamily.Metal4` 的设备在设备创建处产生明确的"does not support
  Metal 4"错误。
- 同时启用两个后端 feature 时门面以 `compile_error!` 失败。
- Windows/Linux 构建不变：同名公共类型、无新增依赖。
- `cargo test -p moonfield-rhi-metal` 在合格硬件上运行离屏冒烟测试，否则
  优雅跳过。
- 边界门禁（`verify_rhi_boundary.py`）在扫描 metal 源码的情况下通过 ——
  任何公共签名都不出现 `objc2`/`MTL*`。

## Risks

- **Slang 的 Metal target 官方标记为实验性。** vertex、fragment、compute、
  task、mesh 阶段列为支持；引擎自身的 ray-query 与 autodiff kernel 能否编译
  成 `metallib` 要等第 2/4 阶段验证。兜底：在验证之前把 rt 与 ml 在 macOS
  上保持能力门控关闭。
- **`ResourceDescriptorHeap[]` —— Vulkan 的 bindless 堆语法 —— 无法为
  Metal 目标编译**（实测："unavailable features in entry point ... for
  'metal' compilation target"）。跨目标的通道是 Slang 内建的
  `DescriptorHandle<T>` 作为 root 结构体字段（双端实测）：
  - **SPIR-V**：字段是 root blob 里的 `uint2`（反射偏移处 8 字节）；
    堆槽位索引取低 `u32`（`.x`）。运行时 `DescriptorHandle<T>(uint2)`
    构造可用。
  - **Metal**：字段*不是* root blob 字节。`EntryPointParams` blob 跳过
    handle 字段（uniform 偏移不含它们地紧凑排布），反射将其报告为
    argument-table 的资源绑定（`shaderResource` / `samplerState` 索引）。
    从索引的运行时构造在 Metal 上不可用（构造器的目标族门控将其排除），
    因此 handle 必须是 root 字段，由 API 侧绑定。
  - API 侧绑定是 `MTL4ArgumentTable::setTexture_atIndex` /
    `setSamplerState_atIndex`，取 `MTLResourceID` —— 资源的
    `gpuResourceID`，普通 `u64`。Metal 的 `TextureHandle`/`SamplerHandle`
    是 gpuResourceID 支撑的值；引擎在反射出的表索引处绑定它们。
  - 对引擎的后果：root 布局按后端分歧（Vulkan 是 blob 偏移，Metal 是
    表槽位），整结构 root 镜像不再跨后端 —— 逐 draw 的 root 推送走
    `RootBinder` 解析的 place，binder 增加 texture/sampler 放置
    （Vulkan 为 uniform 偏移，Metal 为表槽位）。索引
    `ResourceDescriptorHeap` 的 shader 源（egui.slang、core_3d.slang）
    迁移为其 root 结构体里的 `DescriptorHandle<T>` 字段。
- **`objc2-metal` 的 MTL4 覆盖**可能落后于后端需要的头文件；缺口用本地
  `extern_class!` 声明补齐，直到上游发布。
- **argument table 的槽位预算**（Metal 4 限制 buffer bind 数量）与 descriptor
  heap 的容量不同；reflection→槽位映射必须显式预算槽位，否则 bindless 模型
  在密集 pass 上吃紧。
- **egui 移植**是 editor 侧独立的一块工作；第 1–4 阶段先交付 macOS 上具备
  离屏/帧循环能力的 RHI，之后 editor 才可用。

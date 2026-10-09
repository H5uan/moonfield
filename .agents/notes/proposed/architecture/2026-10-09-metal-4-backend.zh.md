# Agent Note: Metal 4 backend

Status: proposed

[English](2026-10-09-metal-4-backend.md)

## Problem

支持的目标平台是 Windows 和 Linux。macOS 需要一个原生后端：MoltenVK 无法
暴露 `Device::new` 所要求的设备表（`VK_EXT_descriptor_heap`、mesh shading、
ray tracing），因此 Vulkan 路线在 macOS 上走不通 —— Apple silicon 上的本地
开发目前完全没有 GPU 路径。平台要求：macOS 默认使用最新的 Metal 4
（`MTLGPUFamily.Metal4`，macOS 26 SDK），设备不达标时给出清晰的错误。约束：
下游 crate 消费的是 RHI 的具体类型，因此新后端必须匹配 Vulkan 后端的公共
名称与形状，而不是引入抽象层。

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
- **`objc2-metal` 的 MTL4 覆盖**可能落后于后端需要的头文件；缺口用本地
  `extern_class!` 声明补齐，直到上游发布。
- **argument table 的槽位预算**（Metal 4 限制 buffer bind 数量）与 descriptor
  heap 的容量不同；reflection→槽位映射必须显式预算槽位，否则 bindless 模型
  在密集 pass 上吃紧。
- **egui 移植**是 editor 侧独立的一块工作；第 1–4 阶段先交付 macOS 上具备
  离屏/帧循环能力的 RHI，之后 editor 才可用。

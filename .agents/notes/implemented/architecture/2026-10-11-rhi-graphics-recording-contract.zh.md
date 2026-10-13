# Agent Note: RHI graphics recording contract

Status: implemented

[English](2026-10-11-rhi-graphics-recording-contract.md)

## Problem

[契约收窄](2026-10-11-rhi-sync-memory-contract-narrowing.zh.md)统一了两后端的内存、同步与
compute 录制；光栅（graphics）表面仍存在分歧：

- `GraphicsPipeline::new` 的参数个数与顺序不同 —— Vulkan 是
  `(device, color_format, vertex_shader, fragment_shader)`，Metal 是
  `(device, module, vertex_entry, fragment_entry, format)`。
- `RenderPassDesc` 不是同一个类型：Vulkan 带 `layer_count`，Metal 不带，
  且 Metal 静默忽略 `depth_attachment`。
- Metal 重建 render encoder 时不重放视口（录制过的视口在 encoder 重建后
  丢失）。
- `TextureView` 访问器不对等：Metal 暴露 `width`/`height`/`format`，
  Vulkan 一个都没有。

这些是把光栅消费方（`Core3d` pass、egui）挡在 Metal 后端之外的全部条目 ——
也是 [Metal 提案](../../rejected/architecture/2026-10-09-metal-4-backend.zh.md)
editor 阶段的前置。它们在此表面之外还需要的东西（离屏/深度目标、egui 的
Metal 后端、时间戳）归该提案与
[分层提案](../../implemented/architecture/2026-10-10-rhi-interface-and-backend-layering.zh.md)
所有，不在此重复。

## Decision

- **管线构造器。** 两后端统一为 `GraphicsPipeline::new(device,
  color_format, vertex, fragment)` 与 `new_with_options(device,
  color_formats, depth_format, vertex, fragment)`，入口名由 `ShaderModule`
  携带（`from_compiled` 记录产出名，`from_msl` 显式命名）；模块缺失入口时
  构造以 `Validation` 失败。Metal 的显式入口构造器已删除。
- **`RenderPassDesc` 对等。** `layer_count` 离开了公共形状：消费方审计发现
  所有调用方都渲染第 1 层 —— 不存在分层渲染消费方 —— 两后端因此单层渲染，
  没有字段被静默忽略。Metal 实现 `depth_attachment`：render encoder 获得
  深度目标（纹理、load/store/clear），管线经 `new_with_options` 声明深度
  格式 —— Metal 在 draw 时校验管线与 pass 的配对。`RenderAttachment`
  在两后端均为 `Clone`。
- **深度状态。** Metal 的 `set_depth_state` 映射到按设备缓存的
  `MTLDepthStencilState`；测试关闭时的写入被强制关闭（Vulkan 的深度写入
  只在测试启用时发生，而 Metal 一旦设置状态就没有关闭开关）。
- **encoder 重建的状态重放。** Metal 的 render encoder 重建（编码过命令后
  的 `push_data`）重放已录制的动态状态：argument table、视口与剪刀
  （render area，或录制的覆盖值）、剔除、深度状态与已绑定的管线。新建
  encoder 不携带状态，逐趟覆盖值在 `begin_rendering` 时复位。
- **重建下的 store op。** Metal 在 encoder 创建时固化 load/store；中间
  encoder 不得丢弃后续 encoder 要重新加载的内容。Metal 后端创建的每个
  render encoder 都 store；`end_rendering` 通过一个尾随 no-op encoder
  （load + 声明的 store）施加声明的 `Discard`，声明值得以保留。
- **`MTL4RenderPassDescriptor` 深度附件。** `setDepthAttachment` 拷贝其
  参数；描述符在拷贝前构建并完整配置。修改 getter 返回值是无效的。
- **`TextureView` 访问器。** 两后端均有 `width`/`height`/`format`；
  Vulkan 的视图从其所有者（`Texture`、`OffscreenTarget`、`DepthBuffer`、
  `Swapchain`）携带范围与格式。
- **光栅目标载体。** 两后端均有 `Texture::new_render_target` 与
  `read_pixels(&self, device)`：带 CPU 读回的可渲染颜色目标，服务测试与
  编辑器视口。Vulkan 的目标在创建时经一次性内部提交转入 `GENERAL`
  （返回即可渲染）；读回经 `CpuReadOnly` 暂存分配拷出。
- **光栅契约测试**（`tests/contract_raster.rs`）以单一拼写经 facade 公共
  API 钉住该表面：Slang 顶点拉取 + fragment、`push_data` root（放置来自
  `RootBinder` 反射）、clear/draw/读回。其断言区分"渲染了"与"只剩清屏色"
  （黑底、红色三角形），空转通过不可能。
- **能力开关。** 无一翻转：四个 `Capabilities` 标志均归 Metal 提案的各
  phase 所有；光栅对等条目是无条件 API 表面，不是可选设备能力。

## Alternatives considered

- **在 Metal 上保留显式入口的管线构造器。** 弃用原因：模块携带入口消除了
  错标入口的门，而管线构造的两种拼写正是本契约要消除的漂移。
- **在 Metal 上实现分层渲染而非删除 `layer_count`。** 弃用原因：没有消费
  方渲染多层；删除是更便宜的对称（审计决定，字段离开了形状）。
- **按后端门控的光栅契约测试。** 弃用原因与 compute 契约相同：门控的测试
  在第二个后端上什么也没钉住；单一拼写才是重点。
- **把 graphics 并入 Metal 提案的各 phase。** 弃用原因：该提案拥有后端能力
  （Metal 能做什么）；本契约拥有公共契约形状（消费方怎么写）。分开落地使
  契约不依赖 macOS 起飞。

## Consequences

- `Core3d` pass 与 egui 只拼写共享的光栅名字（`new_with_options`、
  `set_depth_state`、`push_data`、携带深度的 `RenderPassDesc`）；它们在
  Metal 路径上剩余的缺口（`OffscreenTarget`、`DepthBuffer`、
  `FrameUploader`、egui 的 Vulkan 后端）归 Metal 提案的各 phase。
- Slang graphics 入口在 Metal 上经槽位 0 的 `EntryPointParams` blob 读取其
  `Ptr<T>` root —— 与 compute 相同的形状，因此 `push_data` 在两后端上服务
  两个阶段（[Metal 4 骨架笔记](2026-10-09-metal-4-skeleton.zh.md)记录了
  绑定模型）。
- Metal gpu 测试为每个入口点构造一个模块并经 `push_data` 传递 root；
  手写 MSL 的测试保持其 `[[buffer(0)]]` 声明的缓冲经 crate 内部的
  `set_buffer` 绑定。
- 光栅契约在 Apple 平台的 Metal 上运行，在具备 RHI 所需扩展的 Vulkan 设备
  上运行；其余环境编译并跳过（硬件跳过不是等价性的证据 —— 与 compute 契约
  同一条规则）。
- 首次 macOS 运行（Apple Silicon）验证了两份契约的 Metal 半边 ——
  [同步/内存契约](2026-10-11-rhi-sync-memory-contract-narrowing.zh.md)与本
  契约：完整 gpu 测试与契约套件全部通过，包括 compute 契约的 pending-barrier
  重放。

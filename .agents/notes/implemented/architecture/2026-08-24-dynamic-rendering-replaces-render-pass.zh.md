# Agent Note: Dynamic rendering replaces render pass objects

Status: implemented

[English](2026-08-24-dynamic-rendering-replaces-render-pass.md)

## Problem

RHI 的图形路径一直围绕保留式对象构建 —— `RenderPass` 与 `Framebuffer`。
一个 render pass 硬编码一个颜色附件以及(可选的)`D32Sfloat` 深度附件,
声明 initial/final layout 并手写 subpass 依赖,每次 begin 都要穿一个
`VkRenderPassBeginInfo`。Vulkan 1.3 把 dynamic rendering
(`vkCmdBeginRendering`)升为核心:它彻底移除这些对象 —— 附件按次内联传入,
携带各自的 load/store op 与 clear 值,管线声明的是附件*格式*而不是一个
兼容的 render pass。设备此前已启用 `dynamicRendering`(以及 1.4 的
`dynamicRenderingLocalRead`)。

另一方面,blend/cull/depth 状态按排列组合烘进 `VkGraphicsPipelineCreateInfo`,
正是 Sebastian Aaltonen 在 [No Graphics API][no-gapi] 里反对的 PSO
排列爆炸。Vulkan 1.3 把其中大部分状态变成动态
(`CmdSetColorBlend*`、`CmdSetCullMode`、`CmdSetDepthTest*`),
`VK_EXT_extended_dynamic_state3` 扩展补上 blend 方程与写掩码。沿用博客的
`gpuBeginRenderPass` 形态,本次改动用扁平的逐 pass 描述取代保留式
render pass 对象,并把光栅状态移到逐 draw 的动态命令。

[no-gapi]: https://www.sebastianaaltonen.com/blog/no-graphics-api

## Decision

`RenderPass` 与 `Framebuffer` 被删除;整棵树不再创建任何 `VkRenderPass`
或 `VkFramebuffer`。

- **`RenderPassDesc` + `RenderAttachment`** 取代这两个对象。
  `CommandBuffer::begin_rendering(&RenderPassDesc)` 内联构造
  `VkRenderingInfo`:color/depth 附件自带 image view、layout、load/store op
  与 clear value。`image_layout` 既是渲染期也是结束时的布局 —— dynamic
  rendering 自动完成转换,因此旧的 `SHADER_READ_ONLY_OPTIMAL` 外部 subpass
  依赖 hack 消失;swapchain pass 用 `PRESENT_SRC_KHR`,offscreen pass 用
  `SHADER_READ_ONLY_OPTIMAL`。
- **光栅状态全部动态。** `CullState` 与 `DepthState` 由
  `set_cull_state` / `set_depth_state`(Vulkan 1.3 核心)逐 draw 设置,
  `set_blend_state`(VK_EXT_extended_dynamic_state3,设备上只加载一次)。
  `begin_rendering` 把全部动态状态重置为默认值(blend 关、背面剔除、depth
  关、viewport/scissor = render area),避免 pass 继承陈旧状态 —— 即
  no_gfx_api beginRenderPass 的约定。
- **`GraphicsPipeline`** 用 `color_formats: &[Format]` 与
  `depth_format: Option<Format>` 替代 `&RenderPass`,喂给
  `VkPipelineRenderingCreateInfo`(挂在 pipeline create info 的 pNext 上,
  `render_pass = VK_NULL_HANDLE`)。`PipelineOptions` 只留 `set_layouts`;
  `blend`、`cull_mode`、`depth_test` 全部移除。
- **`Format`** 新增 `D32Sfloat`,让 RHI 中立的格式枚举能指名深度附件。

调用方(编辑器 viewport、egui 后端、窗口渲染器、所有测试)按目标 image
view 构造 `RenderPassDesc`;viewport 的深度 pass 设置
`DepthState { test: true, write: true, GREATER_OR_EQUAL }`(reverse-Z),
egui 在绘制前设置 `BlendMode::PremultipliedAlpha`。

## Alternatives considered

- **公开 API 直接暴露 `vk::RenderingInfo`。** rejected:本次重构的目标正是
  把原始 Vulkan pass 类型挡在 `RenderAttachment`/`RenderPassDesc` 边界之后,
  与博客的描述结构一致,并为 MRT(多个色彩附件)留出空间。
- **blend/cull/depth 继续烘进管线。** rejected:这会保留博客指出的 PSO
  permutation 爆炸;改成动态才让每种 shader 组合只要一条管线。代价只是
  每次 draw 多几条 `CmdSet*` 调用,现代驱动上开销很小。
- **逐个状态的 `cmd_set_*` vs 结构体。** 采用结构体(`CullState`、
  `DepthState`)——no_gfx_api 的 `cmd_set_depth_state` 风格——因为调用方
  更可读,且一个结构体名暗示其所有字段都会被覆盖。

## Consequences

- `VK_EXT_extended_dynamic_state3` 是必需的设备扩展,其 feature 结构体在
  设备创建时启用;否则 blend 的 `CmdSet*` 命令过不了 validation。depth/cull
  动态状态是 Vulkan 1.3 核心,不需要额外 feature。
- 管线现在在创建时指定格式;若目标格式变化必须重建管线(之前通过
  render pass 兼容性同样如此 —— 依赖只是换了个位置)。
- 所有 subpass/外部依赖逻辑消失:dynamic rendering 没有 subpass,final
  layout 转换隐式完成。需要在 pass 之后改布局的渲染器(如 offscreen →
  sampler)通过同一个 `image_layout` 字段表达。

- 编辑器 egui 后端改为用动态命令记录 blend 状态;premultiplied-alpha
  方程编码在 `set_blend_state` 里,窗口与 offscreen 目标共用。
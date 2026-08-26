# Agent Note: In-house egui→Vulkan backend

Status: implemented

[English](2026-08-20-egui-vulkan-backend.md)

## Problem

编辑器的 UI 渲染依赖 `egui-ash-renderer`:它的兼容表锚定了整个 egui 栈(egui / egui-winit / egui_dock / ash / winit 必须跟着它整组升级),UI 资源处在与 Lunar Mare 分离的另一套 allocator 世界,渲染行为掌握在第三方手里。

## Decision

`moonfield-editor::egui_vk` 是编辑器的 egui 后端:基于 Lunar Mare(`moonfield-rhi`)的 `EguiRenderer`,shader 用 Slang 编写。功能规格对齐 egui-wgpu 0.33,并移植为 Vulkan 惯用法:

- API:`update_texture`(全量 + `ImageDelta` 部分更新)、`free_texture`(user 纹理只解绑 descriptor set)、`register_native_texture` / `register_native_texture_with_options` / `update_native_texture` / `update_native_texture_with_options`(外部 image → `TextureId::User`,可缩放目标用 id 稳定的原位换绑)、`texture` 反查、`update_buffers`、`render`(在调用方打开的 render pass 内录制)。
- 纹理:每个 managed `TextureId` 一张独立 `R8G8B8A8_UNORM` image,采样器按 `TextureOptions` 缓存,无 mipmap,不做图集打包。
- 管线:预乘 alpha 混合,20 字节顶点(f32×2 pos、f32×2 uv、打包 u32 sRGB 颜色),screen-size uniform,scissor 由 clip rect × pixels_per_point 得出,u32 索引,顶点/索引 buffer 按 in-flight 帧槽翻倍只增不减,纹理释放延迟到帧槽 fence 之后(编辑器的 free ring)。
- Shader 选项:dithering(interleaved gradient noise,默认开)与 predictable texture filtering(软件双线性,默认关);两个 fragment 入口分别覆盖 gamma(unorm)与 sRGB 目标。

RHI 侧的配套:`GraphicsPipeline` 增加 `PipelineOptions`(blend 模式、cull 模式、descriptor set layout)、`Uint32` 顶点格式、`CommandBuffer` 增加 scissor 与 descriptor set 绑定、`bind.rs` 的 buffer 绑定按 layout 条目声明的类型写入、测试用 `Buffer::read` 与 `OffscreenTarget` 回读支持。

显式不支持:`msaa_samples`、`depth_stencil_format`、`CallbackTrait` paint 回调、多 viewport。callback 的接缝已预留(`render` 在调用方 pass 内录制;`callback_resources` 是预留的共享状态袋),后续接入不需要破坏 API。

## Alternatives considered

- **保留 egui-ash-renderer。** 拒绝:版本锚定与分离的 allocator 世界正是要解决的问题。
- **把 egui-ash-renderer fork 进仓库。** 拒绝:继承它的内部结构等于继承它的形态;基于 Lunar Mare 自写让 UI 资源与场景共用同一套 RHI,并顺带补上 RHI 自身的缺口(pipeline 选项、descriptor 绑定)——这些缺口迟早要补。
- **引入 wgpu 作为中间层。** 拒绝:在 Vulkan 栈上再叠一套 GPU API,自研渲染器落地后没有任何收益。

## Consequences

- egui 栈的版本锚点是 egui_dock 的兼容表;UI 渲染升级只跟随 egui_dock。
- viewport 维持离屏目标 + user texture 架构;离屏 image 经 `register_native_texture` 注册,resize 后经 `update_native_texture` 原位换绑。
- `cargo test -p moonfield-editor --test egui_headless` 无头渲染一帧 egui(文本、user texture、clip rect)并回读像素;CI 上跑 lavapipe,无 Vulkan 驱动的机器上跳过。
- dock 面板与 viewport 的实机显示效果仍需人工确认。

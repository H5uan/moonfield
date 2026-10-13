# Agent Note: Engine layer split out of the Vulkan RHI

Status: implemented

[English](2026-08-26-render-engine-split.md)

## Problem

RHI crate 一个 crate 扮演两种角色。作为 Lunar Mare,它拥有基于 ash 的后端
(`vulkan/*`、`types`、`bind`、`indirect`);作为引擎层,它还拥有提取
(`extract.rs`:`MainEntity`、`extract_cameras`、`extract_with_transform`)、
相机快照(`scene.rs`:`ExtractedView`、`ViewTarget`)、`RenderPlugin` 与窗口
帧循环系统(`vulkan/window.rs`:`extract_windows`、`create_window_surfaces`、
`acquire_window_frames`、`submit_window_frames`)。因此该 crate 依赖
`moonfield-app`、`moonfield-ecs`、`moonfield-camera` 与 `moonfield-window`,
违反了 [Vulkan RHI boundary](2026-08-19-vulkan-rhi-boundary.md) 承诺的纯后端
表面。[Renderer aligned with Bevy](2026-08-24-renderer-bevy-alignment.md)
记录了 `ExtractedView`、`ViewTarget` 与提取系统原先位于 RHI crate 内。

## Decision

RHI crate 改名为 `moonfield-rhi`(Lunar Mare),只保留 RHI 表面:`vulkan/*`
资源与命令代码(device、instance、buffer、texture、offscreen、pipeline、
shader、sync、swapchain、bindless)、`types.rs`、`bind.rs`、`indirect.rs` 与
`RenderDevice` 资源类型。它移除对 `moonfield-app`、`moonfield-ecs`、
`moonfield-camera`、`moonfield-window` 的依赖,并保持为唯一链接 `ash` 的
crate。帧循环的提交与呈现细节作为词汇化助手放进 RHI(`Device::submit_frame`、
`Device::wait_idle`、`Swapchain::format_srgb`、返回
`Error::SurfaceOutOfDate` 的 `Swapchain::acquire_next_image`/`queue_present`),
引擎层因此不链接任何 `ash` 代码。

新 crate `moonfield-render-core`(Selene)拥有引擎层:`extract.rs`(`MainEntity`、
`extract_cameras`、`extract_with_transform`)、`scene.rs`(`ExtractedView`、
`ViewTarget`,以及自 `moonfield-render-feature/src/core_3d/pass.rs` 迁入的
`ViewTargets` 附件映射)、`window.rs`(窗口帧循环:`extract_windows`、
`create_window_surfaces`、`acquire_window_frames`、`submit_window_frames`、
`ExtractedWindow`、`WindowSurfaces`、`WindowFrameDemand`、`WindowSurfaceData`、
`MAX_FRAMES_IN_FLIGHT`)、`plugin.rs`(`RenderPlugin`,经 `RenderDevice::new`
创建 `RenderDevice` 并在渲染世界同一位置插入,保持 LIFO 销毁)。

消费者跟随新布局:`moonfield-render-feature`(Lunaris)相对 Selene 的
`acquire_window_frames`/`submit_window_frames` 排序,并从 Selene 取
`extract_with_transform`、`ExtractedView`、`ViewTarget`、`ViewTargets`;
`moonfield-editor` 从 Selene 取 `RenderPlugin`、帧循环、`WindowSurfaces`、
`WindowFrameDemand`、`MAX_FRAMES_IN_FLIGHT`、`ViewTargets`,而 `egui_vk`
继续使用 RHI 的纯资源(`Buffer`、`BindGroup` ...)。代号 Lunar Mare(RHI)、
Selene(引擎)、Lunaris(功能)分别在各自 README 声明。

## Alternatives considered

**引擎层并入 `moonfield-app`。** 否决:`moonfield-app` 是插件框架,其
`Render`/`RenderPrepare`/`RenderQueue` 标签与渲染器无关;继承引擎层会使应用
依赖 RHI。

**引擎层并入 `moonfield-render-feature`。** 否决:该 crate 是功能层,且已承载
引擎的另一半(`core_3d`、`render_phase`);把引擎并进去恰好保留了本拆分要消除
的混装。

**仅文档约定边界。** 否决:验收标准是结构性属性——`moonfield-rhi` 在没有引擎
依赖的情况下编译——prose 无法强制这一点。

**保留 `moonfield-render` 作为 RHI crate 名。** 否决:存在两个 render 家族
crate 后,`moonfield-render` 会被读作"渲染器"(引擎层)而非 RHI;
`moonfield-rhi` 精确表述了该 crate 的角色。改名是机械操作。

**`RenderDevice` 迁往 Selene。** 否决:它是没有 ECS 依赖的普通资源类型;留在
RHI 使无头一次性消费者(测试中的 `RenderDevice::new`)保持仅依赖 RHI。

## Consequences

- RHI 边界是结构性的:`moonfield-rhi` 在没有 `moonfield-app`/`moonfield-ecs`/
  `moonfield-camera`/`moonfield-window` 的情况下编译;任何在 `moonfield-rhi`
  之外链接 `ash` 的 crate 都会破坏 workspace 规则。
- 窗口帧循环的提交、呈现与格式映射是 RHI 词汇化助手,Selene 不链接 `ash`。
- `RenderDevice` 插入顺序不变,渲染世界 LIFO 资源销毁得以保持。
- `ViewTargets` 暴露 `iter()` 与 `ensure()`,功能层不再需要私有字段访问附件
  映射。
- 窗口行为不变:编辑器与功能 crate 以新名字构建在相同的系统与排序锚点上;
  `cargo test --workspace` 与 headless triangle 冒烟测试通过。
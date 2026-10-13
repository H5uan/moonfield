# Agent Note: Primary-window 3D pass — cameras can draw straight to the window

Status: implemented

[English](2026-09-05-primary-window-3d-pass.md)

## Problem

`RenderTarget::PrimaryWindow` 早已存在于相机词汇表，但没有任何东西往里画：
`prepare_view_targets` 只为 `Viewport` 视图创建离屏附件，`main_opaque_pass_3d` 只录制
到 `ViewTargets`——游戏路径的相机（无编辑器）什么都呈现不出来。窗口帧循环 acquire 了
无人渲染的 swapchain 图像。

## Decision

窗口路径在 pass 层与离屏路径镜像对称，由 surface 提供附件：

- **rhi**：新增一等类型 `DepthBuffer`（独立的 `D32Sfloat` 深度附件，经退役环延迟
  销毁），位于 `offscreen.rs`，复用其中的辅助函数。
- **render-core**：`WindowSurfaceData` 持有一个与 swapchain 同尺寸的 `DepthBuffer`
  （`new` 中创建、`recreate` 中同步调整），经 `depth_view()` 暴露。`extract_cameras`
  写入基础 `WindowFrameDemand`——任何以 `PrimaryWindow` 为目标的相机都会请求窗口帧。
- **editor**：`extract_editor_frame` 把 UI demand 以 OR 方式并入现有值而非覆盖
  （extract 系统按注册顺序执行，`RenderPlugin` 注册在前）。
- **render-feature**：`record_view_pass` 改收 `PassTarget`（color/depth 视图、尺寸、
  color 最终布局）而非 `&OffscreenTarget`；`main_opaque_pass_3d` 新增第二个循环：以
  `PrimaryWindow` 为目标的主视图直接录制到每个 in-progress surface 的 swapchain 图像
  （最终布局 `Present`），并对该 surface 的深度缓冲做深度测试。离屏录制不变。

刻意接受的已知限制：pass 锁定 `VIEW_TARGET_FORMAT`（协商成其他格式如 sRGB 的
swapchain 会以 `error_once!` 跳过，待管线变为按格式键控）；`RenderTarget` 默认值保持
`Viewport`，窗口渲染需经 `CameraTarget` 显式选择；窗口目标相机与编辑器 UI pass（会
clear swapchain 图像）的组合不受支持。

## Alternatives considered

- **场景先渲到离屏目标再 blit 到 swapchain。** 否决：每帧多一次全屏拷贝、多一份
  在途图像，只为避免写一个附件分支；动态渲染下直出路径只是布局差异，不是新管线。
- **把 `RenderTarget` 默认值翻转为 `PrimaryWindow`。** 暂缓：它改变场景加载和编辑器
  生成相机的含义（编辑器依赖 `Viewport` 默认值），属于产品决策，不属于闭环本身。
- **现在就做按格式键控的管线（`HashMap<Format, Core3dPipeline>`）。** 暂缓：
  `Swapchain::new` 本已优先 `B8G8R8A8_UNORM`，单管线今天可服务所有受支持的 surface；
  等真出现仅 sRGB 的目标再加这个 map 也不迟。

## Consequences

- 携带 `CameraTarget(RenderTarget::PrimaryWindow)` 的相机可以把深度测试后的网格直接
  画进窗口——游戏路径脱离编辑器可用。
- `WindowFrameDemand` 变为 OR 累积：相机驱动与编辑器驱动的 demand 组合而非互相覆盖。
- `record_view_pass` 与目标解耦（`PassTarget`），同一份代码服务离屏与 swapchain
  附件。
- 测试：`extract_cameras` 的 demand 行为有 headless 单测；rhi 新增
  `gpu_tests::depth_buffer` 创建/调整尺寸测试；既有离屏 GPU 测试经新签名继续覆盖
  `record_view_pass`。

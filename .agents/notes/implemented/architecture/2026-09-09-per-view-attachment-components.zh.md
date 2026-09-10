# Agent Note: Per-view attachment components

Status: implemented

[English](2026-09-09-per-view-attachment-components.md)

## Problem

[重设计 note](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md) 已拍板"附件是 view 实体组件、挂在持久映射之上"，但落地代码仍保留按键为二值 `RenderTarget` 枚举的全局 `ViewTargets` 注册表，且由 pass 自己解析目标。三个后果：所有 viewport 相机渲染并清除同一张共享离屏图；`PrimaryWindow` 视图对每个进行中的窗口 surface 都录一遍 pass，同一 target 上两个相机双重清除同一张 swapchain image，赢家取决于注册顺序；还冒出了 `clear_orphan_view_targets`——给无 view 认领的 target 抹暗底色的特例，而所有权模型本应让它不可能存在。另外 `Core3dPipeline` 烘死了离屏颜色格式，任何其他格式的 swapchain（如 sRGB）都会静默跳过整个窗口场景。

## Decision

- `render-core` 新增 `prepare_view_attachments`（`PrepareViews`）：exclusive 系统，把每个 `ExtractedView` 的逻辑目标解析成 per-view 的 `ViewAttachments` 组件——color/depth 的 `RenderAttachment` 记录（view、layout、load/store、clear 值）、extent、颜色格式。Viewport 视图画进自己相机的池化 target，总是清除。窗口视图共享窗口的 swapchain image 与 depth buffer，因此相机顺序里的第一个清除、其余 Load——这是"一个 target 多相机"的合成语义，且仅当 depth 需要跨 pass 存活（多于一个相机）时才 Store。目标解析不出（无 acquired image、无池化 target）的 view 拿不到组件，其 pass 自然 no-op。
- `ViewTargets` 改按相机 `MainEntity` 键控——跨帧稳定，不像每帧重建的 render-world view 实体——两个 viewport 相机得到两个 target；`retain_cameras` 退役无 view 认领的 target。`RenderTargetSizes` 迁入 render-core 并采用同样键控。
- 池的 `ensure`（render-feature，持有离屏格式常量）排序 `.before(&prepare_view_attachments)`；pass 系统不再碰 target 枚举——`opaque_pass_3d` 读 `CurrentView` 的 `ViewAttachments`，经[RenderContext 三扇门](2026-09-09-render-recording-context.zh.md)录制。
- `Core3dPipelines` 是按格式键控的映射；`prepare_core_3d_pipeline` 为本帧各 view 解析出的每种格式各建一个变体（离屏格式、主 surface 格式）。`Opaque3d` item 携带 `pipeline: Format`，由 queue 阶段按 view 的 target 盖戳；`DrawMesh` 对映射解析变体——逐 draw 绑管线是冗余的，bind 经 tracked pass 去重。sRGB swapchain 的跳过分支删除：sRGB target 获得自己的管线变体，场景正常绘制（硬件负责编码）。
- `clear_orphan_view_targets` 与 `record_clear_pass` 删除；无认领的池化 target 由 `retain_cameras` 退役，无 viewport view 的编辑器面板显示上一帧画面。
- `TextureView` 克隆共享底层视图且永不拥有，附件记录可以无生命周期负担地复制 view；`RenderAttachment`、`Format` 补上映射与记录所需的 derive。

## Alternatives considered

- **窗口视图向所有进行中 surface 广播（旧循环）。** 否决：每帧记录 views × windows 遍 pass，多相机行为随注册顺序漂移；在窗口携带 render-world 身份之前，`PrimaryWindow` 解析为按 main entity 排序的第一个进行中 surface（`WindowSurfaces::primary`）。
- **清除策略作为相机数据（Bevy 的 `ClearColorConfig`）。** 延后：首清后续 Load 的规则不需要动 `Camera` API 就能确定性地定义合成；逐相机覆盖等出现消费者再加。
- **item 携带 shader 键控 cache 的管线 id。** 按[重设计 note](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md)延后：本 phase 如今每种格式只有一条管线，格式戳是格式真正需要的最小键；shader/图形状态 cache 随多材质一起落地。

## Consequences

- 重设计点名删除的三个特例消失：无孤儿清理系统、无共享 viewport target 冲突、无窗口格式跳过。
- 两个相机指向主窗口时确定性地合成画面，而非双重清除；两个 viewport 相机各画各的 target。
- 编辑器把 viewport 面板尺寸与纹理查找都键到 viewport 相机实体上，per-camera target 端到端可用。

## Verification

- `cargo fmt`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（含 GPU 回读、深度遮挡、radix sort、sort pass 顺序测试）全部通过；`python3 scripts/verify_agents.py` 通过。

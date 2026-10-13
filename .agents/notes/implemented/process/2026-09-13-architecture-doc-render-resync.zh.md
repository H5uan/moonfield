# Agent Note: resync architecture.md's renderer section to the set-chain redesign

Status: implemented

[English](2026-09-13-architecture-doc-render-resync.md)

## Problem

`docs/architecture.md` 的渲染器一节仍在描述重构前的渲染管线,与已随代码落地、记录在
[render set chain and per-view schedule](../architecture/2026-09-09-render-set-chain-and-per-view-schedule.md)
和
[per-view attachment components](../architecture/2026-09-09-per-view-attachment-components.md)
中的模型相矛盾。文档写的是渲染世界的三个 schedule(`RenderPrepare`/`RenderQueue`/
`Render`),而代码只有一个由 set 链定序的 `Render` schedule;文档描述了已被移除的
`Core3dFrame`/`Core3dView` 类型;文档声称 Core3d pass 锁定 `VIEW_TARGET_FORMAT`、
sRGB swapchain 会被跳过,而 `Core3dPipelines` 已是按格式键控的映射;文档还说离屏
target 的终态布局是 `SHADER_READ_ONLY_OPTIMAL`,而
[unified image layouts](../architecture/2026-08-26-unified-image-layouts.md)
让所有非 swapchain 图像统一使用 `GENERAL`。

## Decision

就地改写漂移的段落,保持该节的结构与详略:tick 和 `App::render` 的描述现在写明
`ExtractSchedule` 和单一的 `Render` schedule 及其
`PrepareAssets → Queue → PhaseSort → PrepareViews → CameraDriver → PostViews →
Submit` set 链;帧回路系统改为括号式描述(acquire 在链之前,
`create_window_surfaces` 在 `PrepareAssets` 之后,submit 在 `Submit` 之后);Core3d
叙述改为经由逐 view 的 `RenderPhase<Opaque3d>` 组件、camera driver 的逐 view
`Core3d` schedule、`prepare_view_attachments` 的 `ViewAttachments` 解析,以及按格式
键控的 `Core3dPipelines`;布局叙述写明统一 `GENERAL` 的实际行为,swapchain 图像保持
`PRESENT_SRC_KHR`;逐 draw 数据的描述与
[view uniforms](../architecture/2026-09-04-view-uniforms.md)
中 vertex-pulling 的 `DrawData`/`ViewUniforms` 记录一致。核查后仍与代码一致的章节
(extraction、`FrameContext` 与退休环、编辑器的 egui 系统)原样保留,仅补写 egui
系统所在的 set。

## Alternatives considered

- **整节重写渲染器部分。** 否决:其中大部分仍与代码一致;整节重写会扰动准确的行文,
  也会掩盖本次重构真正改动的内容。
- **改为指向 Agent Notes,不再复述机制。** 否决:architecture.md 是机制的汇总文档;
  笔记记录决策,文档必须成行文地承载决策落地后的机制。

## Consequences

- 渲染器一节重新与已落地的调度结构、view 模型、管线键控和布局行为一致。
- 审计中发现代码里两处过时的文档注释
  (`crates/moonfield-render-feature/src/shader.rs` 中的 `RenderPrepare`、
  `crates/moonfield-render-feature/src/lib.rs` 中的 `Core3dFrame`),留待改动代码时
  一并修正;本次只改文档。
- 文档漂移没有 CI 门禁;发现它仍依赖评审和定期审计。

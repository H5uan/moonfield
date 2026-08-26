# Agent Note: Lightweight render-phase framework

Status: implemented

[English](2026-08-26-render-phase-framework.md)

## Problem

core 3D pass 直接录制 mesh:`main_opaque_pass_3d` 导入 `ExtractedMeshes` 与
`PreparedGpuMeshes`,mesh queue 逻辑住在硬编码的 `Opaque3dPhase::queue` 里,由
camera driver 在构造 `Core3dFrame` 时调用。添加第二种 draw 种类(透明、
splat rasterization)意味着要改 pass 和帧结构。
[Renderer aligned with Bevy](2026-08-24-renderer-bevy-alignment.md) 曾为
one-pass/one-phase 帧否决过 draw-function registry;现在 pass 必须停止指名
mesh 类型,registry 被采纳。

## Decision

`moonfield-render-core`(Selene)在 `render_phase.rs` 拥有最小 phase 框架——
Bevy `RenderPhase`/`DrawFunctions` 的形态,但没有 `RenderCommand` chain:

- `PhaseItem` — 纯排队数据,带 `Ord` 排序键与 `DrawFunctionId`。
- `DrawFunction<P>` — 从 `(&World, &P, &CommandBuffer)` 录制一个 item 的 GPU 工作。
- `DrawFunctions<P>` — render-world 资源;feature 在 plugin build 注册一次
  draw function,pass system 按 item 的 id 查表。
- `RenderPhase<P>` — 一个 view 的已排序 item 集合:`Default`(空)、`add`、
  `sort`、`items`。
- `OrderedFloat` — `f32` 排序键的 `Ord` 包装(`total_cmp`)。

在 `moonfield-render-feature`(Lunaris)中,mesh feature 把 `DrawMesh`(pipeline +
vertex/index 绑定 + push constants + indexed draw,含 revision-matched GPU
buffer 校验)注册进 `DrawFunctions<Opaque3d>`,其 `queue_opaque_3d` system 用
仍存活的 mesh item 填充每个 view 的 `RenderPhase<Opaque3d>`,在 queue 时计算
camera-space depth 与最终的 view-projection × model 矩阵。`Core3dFrame` 保留
camera-driver 职责(primary view 排序、per-target 分组),并暴露 `views_mut`
供 queue system 使用;`build` 不再 queue 任何内容。opaque pass 清除附件、设置
pass 级 viewport/depth/cull 状态,把每个 item 分发给其注册的 draw function——
不导入任何 mesh 类型。

## Alternatives considered

**完整 Bevy `RenderCommand` chain(宏组合子 + batching)。** 否决:command
buffer 单线程,帧内只有一个 pipeline;组合子机制为并行录制与 multidraw 而存在,
moonfield 用不到。

**`enum Drawable` 覆盖所有 draw 种类,pass 内 match。** 否决:pass 仍会指名
每种 draw,添加一种仍需改 pass。

**item 自带 draw 闭包。** 否决:registry 让 draw function 按 phase 集中存放
(feature 注册一次),并让 item 保持 `Copy`——Bevy 的形态。

## Consequences

- 添加 draw 种类是注册而非修改:一个 phase item 类型 + 一个 queue system + 一个
  draw function,pass 不动。
- pass 不再导入 `ExtractedMeshes`/`PreparedGpuMeshes`;mesh feature 通过
  `DrawMesh` 拥有自己的 pipeline 与 per-draw 资源。
- queue system 在 `RenderQueue` 中排在 `prepare_core_3d_frame` 之后
  (`after`);`Core3dFrame::build` 创建空 phase。
- queue 时计算 `mvp` 使用的 `RenderTargetSizes`/初始尺寸回退与
  `prepare_view_targets` 一致,投影几何与已准备的附件匹配。
- `RenderPhase` 带 `Debug`/`Clone`/`PartialEq` derive 约束,使 `Core3dFrame`
  既有 derive 保持成立;`Default` 手写,phase 无需 `P: Default`。
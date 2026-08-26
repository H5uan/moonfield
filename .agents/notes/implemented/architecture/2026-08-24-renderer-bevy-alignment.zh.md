# Agent Note: Renderer aligned with Bevy

Status: implemented

[English](2026-08-24-renderer-bevy-alignment.md)

## Problem

应用已经有 render world 和手写 extraction，但 render schedule 仍在 main
world 上执行。编辑器在一个 exclusive function 中同时持有 CPU 交互状态、GPU
资源、mesh 上传、scene queue、swapchain 帧控制和 presentation。render-world
entity 每帧重建，却没有稳定指向 main-world 来源的标识；可变资产也没有可用于使
GPU 准备数据失效的 revision。

这使 render world 只能被观察，不能成为实际渲染输入：viewport 在记录 GPU
命令时仍查询 main-world 资产和相机，也没有针对 camera view、prepared mesh、
opaque draw 顺序或 acquire → pass → submit 帧边界的 render-world 契约。

## Decision

`App::render` 执行五个明确阶段：

```text
PreRender(MainWorld)
→ 清理 render snapshot entity 并执行 MainWorld → RenderWorld extraction
→ RenderPrepare(RenderWorld)
→ RenderQueue(RenderWorld)
→ Render(RenderWorld)
```

main-world schedule 通过 `App::add_systems` 注册；render-world schedule 通过
`App::add_render_systems` 注册。render-world resource 不受 snapshot 清理影响，
并持有所有跨帧 GPU 状态。`RenderDevice` 只存在于 render world。

这些粗粒度 render stage 采用本地 Bevy `0.20.0-dev` 生命周期中适合本项目的部分，
但不复制完整的 `SystemSet` 图。`RenderPrepare` 把抽取出的 CPU 数据转换为持久 GPU
资源，`RenderQueue` 构造每帧 view 与 phase 工作，`Render` 记录并提交命令。
moonfield 的 scheduler 没有 `SystemSet`，且所有阶段都在单线程执行，因此这些阶段
使用独立的 render-world schedule。

`HierarchyPlugin` 也在 `PreRender` 中执行 transform propagation。
`editor_prepare` 排在 `ensure_global_transforms` 之前，随后执行
`propagate_transforms`，因此 orbit camera 的修改会在 main-world snapshot 抽取前
写入 `GlobalTransform`。

每个抽取出的场景 entity 都带有 `MainEntity`，作为其 main-world 来源的稳定
key。`moonfield-camera` 持有场景侧的 `Camera`、`PrimaryCamera`、
`CameraTarget`、`RenderTarget` 以及投影和视图数学，并且不依赖 Vulkan RHI。
`moonfield-render-core`(Selene)消费这些类型，并保留 render-world 的
`ExtractedView`、`ViewTarget` 和 extraction system。相机抽取会记录相机参数、传播后的 transform、
来源标识和逻辑 target。`CameraTarget` 仍是独立的运行时 component，因此场景相机
序列化格式不变。

`Assets<T>` 在资产插入或被可变访问时分配 `AssetRevision`。mesh 和 splat
extraction 只复制被可渲染 entity 引用且仍存活的资产。`ExtractedMeshes` 在
render world 中跨帧保留，只在 revision 变化时替换 CPU 几何数据。
`PreparedMeshes<T>` 将 GPU 数据与来源 `AssetId`、revision 关联；过期或不再被
引用的条目不会参与绘制。

编辑器由两个 owner 组成，并通过一个有界 bridge 连接：

- `EditorMainState` 位于 main world，持有 egui 输入、dock、selection、场景编辑、
  orbit camera 和 gizmo 状态。
- `EditorRenderState` 位于 render world，持有 `WindowRenderer`、offscreen
  viewport、egui Vulkan renderer、frame slot 和延迟纹理销毁状态。
- `PreparedEditorFrame` 将最新 UI shape 与纹理更新从 `PreRender` 送入 render
  world。反向的 render feedback 只传递 viewport texture id 和已完成帧数。

render schedule 通过三个有序 system 驱动窗口帧：`editor_acquire`、
`editor_record` 和 `editor_submit`。只有 acquire 调用
`WindowRenderer::begin_frame`，只有 submit 调用 `WindowRenderer::end_frame`。
record 失败仍会进入 submit，确保 acquired image 与 command-buffer 状态在下一帧前
被关闭。

`moonfield-render-feature` 是位于 `moonfield-rhi` RHI 与
`moonfield-render-core` 引擎层之上的高层 render-feature 层，对应 `bevy_pbr`
这类 Bevy feature crate。其 `RenderFeaturePlugin` 在
`RenderPrepare` 中根据 `ExtractedMeshes` 准备 `PreparedGpuMeshes`，随后在
`RenderQueue` 中构造 `Core3dFrame`。GPU mesh buffer 以来源 `AssetId` 和
`AssetRevision` 为 key，生命周期独立于 editor viewport，并可供任意 render-world
consumer 使用。每个 `Core3dView` 持有一个已排序的 `RenderPhase<Opaque3d>`；mesh feature 的
`queue_opaque_3d` 用仍存活的 mesh item 填充它，并把 `DrawMesh` 注册进该 phase
的 `DrawFunctions` registry——pass 把 item 分发给已注册的 draw function，因此
从不指名 mesh 类型（见
[render phase framework](2026-08-26-render-phase-framework.md)）。编辑器 viewport 消费指向 viewport 的 primary view，把已经准备好的
opaque phase 记录到持久 offscreen target，再由 egui pass 采样。

## Alternatives considered

**完整的 Bevy sub-world 同步和 retained render entity。** 否决，因为 moonfield
每帧重建小型 render snapshot。`MainEntity` 加持久 resource 已能为当前 cache 提供
所需身份，不需要 observer、双向 map 或 `SubEntity` 生命周期机制。

**通用 RenderAsset 框架。** 否决，因为当前 renderer 只有一条 prepared mesh
路径和一条 splat metadata 路径。资产 revision 与具体 extraction cache 已能处理
失效，不需要依赖图、上传预算、重试队列或设备恢复策略。

**把 Vulkan 窗口所有权拆成多个 ECS component。** 否决，因为
`WindowRenderer` 负责维持 swapchain、command buffer、fence、semaphore、surface
与 device 的生命周期顺序。system 只驱动其帧边界，不暴露内部所有权约束。

**render graph 与完整 draw 机制。** 对于当前帧——一个 Core3d scene pass 和一个
editor UI pass——被否决：有序 system、每 view 一个已排序 phase 与 `Core3dFrame`
已能表达数据流，不需要 graph node、binned phase、multidraw、batching 或
`RenderCommand` chain。轻量 draw-function registry 被单独采纳，用于把 pass 与
mesh feature 解耦——见 [render phase framework](2026-08-26-render-phase-framework.md)。

**完整复制 Bevy 0.20 render 生命周期。** 否决，因为 `SubApp` 流水线、持久
render entity 同步、`RenderStartup` 设备恢复和完整的 `RenderSystems` set 图解决的
是 moonfield 当前并不存在的规模与线程需求。独立 render world 和粗粒度 schedule
保留了适用的所有权与顺序边界，无需先让 Vulkan 对象实现 `Send` 或扩展 ECS scheduler。

**在同一次迁移中加入 Vulkan work graph 或 PBR。** 否决，因为两者都不是建立
world 所有权、资产准备、view extraction 或 pass 顺序的必要条件。当前 opaque
路径保留现有 flat-color shader 和 CPU 记录的 indexed draw。

**把 camera API 留在 `moonfield-rhi`。** 否决，因为场景和编辑器代码不应从
Vulkan RHI 获取相机 component。独立 camera crate 让相机数据和数学可被复用，
同时保持 RHI 和 feature crate 只沿一个方向依赖它。

## Consequences

render command recording 不再读取 main-world 相机或 mesh 资产。orbit camera 编辑
和 pre-render transform propagation 都发生在 extraction 前，因此同一帧的 snapshot
会看到更新后的 transform。render world 是 Vulkan 状态的唯一 owner，编辑器的
main-world preparation 不依赖 Vulkan device。

`Assets::get_mut` 会保守地推进资产 revision，即使调用方没有修改值。这可能导致
一次不必要的重新上传，但无需 mutation guard 或 asset event，就能避免静默复用
过期 GPU 数据。

bridge 只保留最新的 prepared editor frame。被替换的帧会合并纹理更新，因此
render 侧跳帧时不会丢失字体或用户纹理 delta。窗口最小化或 out-of-date 时，待处理
工作会保留到 acquire 成功。

opaque phase 只覆盖 flat-lit mesh。它不包含 material、transparency、shadow、
自动 batching 或 GPU-driven draw——新的 draw 种类注册 phase item、queue system
与 draw function，而不是修改 pass。`ViewTarget` 选择 primary window 或 editor viewport；
持久 offscreen Vulkan target 仍由 `EditorRenderState` 持有。

GPU mesh preparation 属于 `moonfield-render-feature`，viewport 继续持有自己的
target 和 graphics pipeline。低层 Vulkan 对象仍是普通 Rust owner 或 render-world
resource；ECS 驱动其生命周期阶段，而不是把对象拆成 entity。
每个 prepared GPU mesh 都持有共享 device，以免 buffer 销毁依赖 render world 的
resource 析构顺序。

render-feature crate 的默认 feature 是 `mesh`；`splat` 为 opt-in 且依赖
`mesh`。`rt` 和 `gi` 占位模块已移除。这一结构先确立层级边界，不在公开契约
尚未形成时提前把每种算法拆成独立 crate；`moonfield-pbr` 一类 feature crate
可在对应契约形成后引入。

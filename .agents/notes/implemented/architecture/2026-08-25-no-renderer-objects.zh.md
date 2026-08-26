# Agent Note: No renderer objects — data-driven frame orchestration

Status: implemented

[English](2026-08-25-no-renderer-objects.md)

## Problem

渲染栈围绕拥有对象的类型成长起来:`WindowRenderer` 通过 `begin_frame`/`end_frame`
方法契约驱动 swapchain 帧循环,编辑器的 `Viewport` 自己拥有管线并录制场景 pass,
egui 后端是单个 `EguiRenderer`,`EditorRenderState` 把三者胶合成一个 god object,
由三个手工排序的系统通过 take-out/put-back 槽位变更。Bevy——本 workspace 遵循的
架构——根本没有 `Renderer` 类型(对照 bevy 0.20-dev 核实):GPU 单例是扁平资源,
per-window 状态是 ECS 数据,帧流程是系统调度。对象形态把帧编排藏进 schedule
看不见的方法契约,迫使调试接缝(`MOONFIELD_EDITOR_SCENE_ONESHOT`、
`..._SKIP_UI`)存在,并阻碍 bevy 风格的方向(pipelined rendering 需要 render
world 是纯数据)。

## Decision

任何地方都不存在名为 `*Renderer` 的类型来拥有帧循环;渲染 = 资源 + component +
系统。

- **窗口帧**(`moonfield-render-core/src/window.rs`):每帧窗口快照作为
  `ExtractedWindow` component 抽取(`extract_windows`);持久的 surface/swapchain/sync
  状态位于以 `MainEntity` 为 key 的 `WindowSurfaces` 资源里(render world 每帧清空
  entity,所以持久 GPU 状态必须是资源)。帧循环是三个公开系统——
  `create_window_surfaces`(RenderPrepare)、`acquire_window_frames` 与
  `submit_window_frames`(Render)——由 `RenderPlugin` 注册为排序锚点,其他插件
  以 `.after()`/`.before()` 挂接。Acquire 由 `WindowFrameDemand` 资源(抽取时写入)
  门控,并跳过零尺寸窗口,因此不会有任何 pass 未录制的图像被呈现。
- **场景 pass**(`moonfield-render-feature/src/core_3d/pass.rs`):flat-lit 管线是
  懒创建的 `Core3dPipeline` 资源,离屏附件是经 `RenderTargetSizes`(编辑器抽取写入)
  定尺寸的 `ViewTargets` 资源,`main_opaque_pass_3d` 是普通 `Render` 系统,把每个
  view 的 `RenderPhase<Opaque3d>` 录进窗口帧的 command buffer。
- **编辑器**(`moonfield-editor`):`EditorRenderState`、`EditorBridge` 与 `Viewport`
  已删除。main world 暂存 `PendingEditorFrame` 资源;`extract_editor_frame` 把它移进
  render world(并入未消费帧,egui texture delta 永不丢失),并设置
  `RenderTargetSizes` + `WindowFrameDemand`。`EguiRenderer` 拆成三个 render-world
  资源——`EguiPipeline`(管线、layout、sampler 缓存、`EguiOptions`)、`EguiTextures`
  (texture 表、无延迟释放环、上传池)、`EguiFrameResources`(per-slot buffer)——由
  `prepare_egui_frame` → `egui_pass` → `editor_frame_done` 系统驱动,相对于窗口与
  场景锚点排序。Render→main 反馈(viewport texture id、已呈现帧数)是仅剩的信道,
  一个克隆进两个 world 的 `EditorFeedbackChannel` `Arc`。
- **资源销毁是 LIFO**(`moonfield-ecs`):`World` 的资源存储按首次插入逆序销毁。
  Vulkan 包装持有裸 `ash` 句柄,因此由更早插入的 `RenderDevice` 创建的 GPU 对象必须
  在它之前销毁;在确定性化之前,HashMap drop 顺序在关闭时造成过 access violation。

## Alternatives considered

- **保留这些对象,只改名。** 否决:问题从来不是名字,而是帧编排活在 schedule
  看不见的方法契约里;改名把 take/put 槽位和调试接缝原样留下。
- **保留 render-world entity,把窗口状态放进 component(照搬 bevy 0.20)。** 推迟:
  `App::render` 每帧清空 render-world entity,持久 GPU 状态今天无法放在 entity 上;
  `MainEntity` 为 key 的资源映射与 bevy pre-0.20 的 `WindowSurfaces` 形态一致,
  entity 保留是独立课题(它也解锁 pipelined rendering)。
- **引入 system set 排序。** 推迟:`before`/`after` 挂在公开系统函数上覆盖了今天的
  图;第三方需要对一组系统排序时再引入 set。
- **让每个 Vulkan 包装持有 `Arc<Device>`,而不是依赖 LIFO drop。** 推迟:LIFO 存储
  用一处局部改动修复了观察到的关闭崩溃,并镜像 Rust 结构体字段的 drop 顺序;
  如果资源插入顺序对此变得不可靠,per-object `Arc` 仍是加固选项。

## Consequences

- 帧流程作为数据可读:extract → prepare → queue → acquire → passes → submit,
  每一步都是命名系统,任何插件都可对其排序。`MOONFIELD_EDITOR_SCENE_ONESHOT` 与
  `MOONFIELD_EDITOR_SKIP_UI` 已删除;`MOONFIELD_EDITOR_DUMP_VIEWPORT` 作为系统保留。
- 插件通过注册系统与资源组合(`RenderFeaturePlugin` 添加 `main_opaque_pass_3d` 而
  编辑器无需知情),splat/rt/gi 功能应复制该形态。
- 消费者必须遵守类型系统无法强制的两条契约:持久 GPU 状态住在资源里(entity 每帧
  重建);从 `RenderDevice` 创建的资源必须在其后插入,LIFO drop 才会先销毁它们。
- `MeshRenderer` 保留其名字——它是 per-entity component(bevy 的 `Mesh3d`
  对应物),不是拥有帧循环的对象。
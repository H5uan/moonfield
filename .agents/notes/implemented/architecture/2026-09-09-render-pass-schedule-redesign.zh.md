# Agent Note: Render pass schedule redesign

Status: implemented

[English](2026-09-09-render-pass-schedule-redesign.md)

## Problem

一帧的 pass 结构曾全部住在一个系统里。`moonfield-render-feature` 的 `main_opaque_pass_3d` 惰性构建唯一的 `Core3dPipeline`、遍历 view、内联录制 opaque pass；与相邻系统的顺序靠函数名锚定，editor 在编译期耦合这些名字。在 opaque pass 前面加一个 compute pass 要动四个 crate 的约八个文件；加一个后处理 pass 动六个。每个 target 只渲染一个 view——第一个 primary view——且 opaque pass 丢弃深度附件，下游无法做深度测试。

phase 机制（[render-phase framework](2026-08-26-render-phase-framework.md)）带着[被取代的优化 note](../../rejected/architecture/2026-08-31-render-function-optimization.md) 所记录的每帧开销与单管线限制：HashMap dispatch、逐 item 资源查找、排序混在 queue 系统里、`Core3dFrame` 每帧 clone。

抽取是 `App` 上按注册顺序排列的闭包列表，schedule 是 `App` 的字段——没有系统能运行 schedule，抽取也无法排序或过滤。

[Gaussian Splatting 路线图](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md) 的 M3 要求把 tile-based forward 录制成资产上传与 view target 之间的一个 pass。旧形状里 compute pass 链没有落点、没有逐 view 执行、没有附件共享；本改造就是为 M3 造这个家。

## Decision

渲染层运行在 Bevy 0.20-dev 删除自家 `RenderGraph` 之后收敛出的 schedule 模型上（其 slot 系统从未被使用；schedule 即子图）：

- **schedule 是 world 数据。** moonfield-ecs 持有 `Schedules` resource 与 `World::run_schedule(label)` 作为唯一的运行原语；`App` 的方法是包装。exclusive system 类型（`fn(&mut World)`）与 `SystemState<P>` 同 crate 落地（[schedules as world data](2026-09-09-schedules-as-world-data.zh.md)）。
- **抽取是 schedule。** `App::render` 把主世界停进 `MainWorld(World)` resource，清空渲染世界实体（资源保留），运行 `ExtractSchedule`——系统经 `Extract<T>` 参数读主世界——换回世界，再运行 `Render`。闭包列表已删除；每帧实体重建保留（[extract schedule](2026-09-09-extract-schedule.zh.md)）。
- **单一 `Render` schedule，按 set 排序。** 帧定序器的 acquire 是非公共入口；其后 `PrepareAssets` → `Queue` → `PhaseSort` → `PrepareViews` → `CameraDriver` → `PostViews` → `Submit`。`RenderPrepare` 与 `RenderQueue` 标签已删除。
- **逐 view 执行。** exclusive 系统 `CameraDriver` 按 `(Camera::order, entity)` 排序 view（`Camera` 携带 `order` 字段），把 `CurrentView` 指向每个 view，运行该 view 的 schedule。注册的 view schedule 只有一个——`Core3d`，内含锚 set `Core3dOpaquePass`；splat 链 `[sort → forward → composite]` 以 `.after(Core3dOpaquePass)` 锚入——这个顺序同时是将来 mesh+splat 深度合成需要的顺序（[set chain and per-view schedule](2026-09-09-render-set-chain-and-per-view-schedule.zh.md)）。
- **`RenderContext` 是封闭的录制面。** 一个 `SystemParam`，三扇类型化的门：`begin_rendering(RenderingDesc) -> TrackedRenderPass`、`compute() -> ComputeRecording`、`barrier(before, after, hazard)`。没有裸命令缓冲句柄；设备访问以独立 `Res` 参数组合。阶段状态机自动插入 rhi 的全局无资源 barrier——门切换时插入、连续 dispatch 之间插 `COMPUTE → COMPUTE`；同一 rendering 内的 draw 不插；手动 `barrier` 照录并更新状态机。帧命令缓冲保持单条（[render recording context](2026-09-09-render-recording-context.zh.md)）。
- **附件是 view 实体组件，压在持久 map 之上。** GPU 资源留在 resource map（`ViewTargets`、`WindowSurfaces`）；view 实体携带逐帧链接，load/store 由各 pass 声明。深度 store 策略是 `Core3d` 的显式决定（[per-view attachment components](2026-09-09-per-view-attachment-components.zh.md)）。
- **绘制机制是 `Vec` 索引**（吸收被取代的 note）：`Vec` 索引 dispatch 与带 phase 类型参数的 `DrawFunctionId<P>`、`RenderCommand<P>` 加基于 `SystemState` 的 `RenderCommandState`、统一 `PipelineCache`——graphics 以 `(ShaderHandle, GraphicsStateKey)` 为键、compute 以 `ShaderHandle` 为键、在 `PrepareViews` 同步创建——以及薄 `SortedPhasePlugin<P>` 注册面。`Core3dFrame` 已溶解为 view 组件。
- **GS forward 渲到自己的 image。** blend kernel 写一张 RGBA16F storage image（rhi 的一个加法构造器，`SAMPLED|STORAGE`）；composite pass 做 tonemap 写入 view target。逐 view 产物（tile lists、transmittance）放在按 `ViewTargets` 方式 key 的 `SplatViewArtifacts` resource map 里。
- **editor 解耦。** editor overlay 锚 `PostViews` set，不再锚 render-feature 的系统名。

## Alternatives considered

- **运行时 render-graph 对象。** 落选：参考实现删掉了自己的图运行器——slot 系统从未被使用、schedule 已承担子图角色——且图运行器是 ECS 调度器之外的第二套执行引擎，还要维护 node 状态机与运行时 slot 匹配。
- **保留三个渲染 schedule、set 放在里面。** 落选：schedule 标签与 set 会成为同一排序概念的两种拼法；逐 view 运行 schedule 仍然要求 schedule 是 world 数据，这个拆分什么都没省，还推迟了 M3 需要的机制。
- **现在就上 `PendingCommandBuffers`。** 落选：moonfield-ecs 串行执行系统、各自独占 world 访问，延迟 finish encoder 没有收益；它还会重排[帧命令缓冲](2026-09-06-frame-context-owns-frame-command-buffer.zh.md)的提交路径，而那条路径归 [timeline pacing](2026-08-28-timeline-frame-loop.zh.md) 所有。
- **`RenderContext` 开一扇裸 `cmd()` 门。** 落选：Bevy 的 `command_encoder()` 逃生舱安全是因为 wgpu 自动同步；moonfield 的 RHI 是 bindless GPU 指针之上的裸 Vulkan，裸门等于重新导出整个命令缓冲表面，把 barrier 纪律散给每个调用点。封闭的类型化门让同步可见、录制可替换。
- **按资源自动 barrier。** 落选：bindless 描述符是裸 GPU 指针——rhi barrier 刻意无资源列表，wgpu 式读写跟踪在不重造资源模型的前提下表达不出来。阶段作用域的自动 barrier 才是合身的形态。
- **dispatch 之间只留手动 barrier。** 落选：当前与规划中的每条 compute 链（radix sort 各 pass、projection → bucketing → sort → blending）都是真实的写后读链；把频率最高的同步场景留给手动，自动化就丢掉了大半价值。
- **GS forward 直写 viewport target。** 落选：离屏 color 是 `COLOR_ATTACHMENT|SAMPLED`——没有 storage 用途位、8-bit 精度、没有 tonemap 点；RGBA16F 中间图以一个 rhi 加法构造器为代价，对齐参考实现的 float 管线。
- **batching、binned phase、mesh transparent phase、更多 view schedule。** 落选：都没有客户；被取代的 note 与本改造都拒绝没有调用方的机制。

## Consequences

- 一个 pass = 新文件 + 一次注册调用，相对 opaque pass 有序，不改 render-feature 核心——由[验收记录](2026-09-09-render-pass-schedule-acceptance.zh.md)端到端 GPU 验证；该记录同时记下唯一有意保留的缺口（`in_set` 成员资格在注册时定型，因此 splat 链锚在 opaque pass 系统上）。
- editor viewport、PrimaryWindow 直绘、egui 合成行为不变。
- GS forward 链有家：`Core3d` 里的逐 view 系统、产物 map、composite pass——[GS 路线图](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.zh.md) M3 措辞与已交付形状一致。
- `Schedules` 入 world、`World::run_schedule`、exclusive system、`SystemState` 是 render command 同样赖以构建的 ECS 地基；抽取闭包列表已消失。
- queue 系统必须排在 `PhaseSort` 之前；排错的 queue 系统会静默渲染未排序内容——与被取代 note 记录的是同一隐患。
- 自动 barrier 是保守的全局内存 barrier；可能过同步，且 `Stage` 没有附件输出类常量，composite 或 overlay 顺序若暴露附件加载 hazard 会需要补。
- pass 录制在单命令缓冲上 CPU 串行；editor 或 GS 负载若长大到超过它，并行执行器的决策重新打开。

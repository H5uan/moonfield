# Agent Note: Render command dispatch

Status: implemented

[English](2026-09-09-render-command-dispatch.md)

## Problem

draw dispatch 带着被[取代的优化 note](../../rejected/architecture/2026-08-31-render-function-optimization.zh.md)记录过的开销：`DrawFunctions` 是 `HashMap<u32, …>`，每个 item 每帧付一次哈希查找；`DrawFunctionId` 是无类型 `u32`，一个 phase 注册表铸造的 id 传给另一个 phase 也能编译；`Opaque3dDrawFunction` 用手写 newtype 资源把 id 从 plugin 构建递到 queue 系统；`DrawMesh::draw` 逐 item 重读四个资源、绑定不去重（每个 item 重复绑同一条管线）。

## Decision

- `DrawFunctions<P>` 存 `(TypeId, Box<dyn DrawFunction<P>>)` 的 `Vec`；dispatch 按 `DrawFunctionId<P>` 索引——无哈希。`DrawFunctionId<P>` 携带 `PhantomData<fn() -> P>`，id 在编译期绑定到 phase。
- 注册与查找都是类型导向：`register::<C>()` 存命令的 `TypeId`；`id::<C>()` 按类型找回 id，queue 系统直接读 `DrawFunctions<Opaque3d>` 要 `DrawMesh`。`Opaque3dDrawFunction` newtype 消亡。
- 无状态 `RenderCommand<P>` trait 声明 `type Param: SystemParam` 与关联 `render(world, item, pass, param)`；`RenderCommandState<P, C>` 持有 `SystemState<C::Param>`，即注册表存储的对象安全 `DrawFunction<P>`。`DrawMesh` 现在是 `RenderCommand`，其 `Param` 为抽取 mesh、预备 mesh、管线与 draw arena——经 param 每 item 取一次。
- `TrackedRenderPass`（render-core）包装帧命令缓冲，跳过冗余管线绑定。tracking 在每次 `begin_rendering` 时重置；管线的地址在 tracking 窗口内即其身份——pass 录制期间管线是不可变资源（重建发生在 `PrepareViews`），所以这是可靠的。
- `prepare_phase<P>`（泛型 phase 装配）与 `sort_phase<P>` 打包进 `SortedPhasePlugin<P>`：每个 phase 一个插件，在 `Queue` 与 `PhaseSort` 注册 view-phase 管道。`RenderFeaturePlugin` 为 `Opaque3d` 实例化。

## Alternatives considered

- **Bevy 对 `Param` 的 `SystemParam` 只读约束。** 落选：强制它需要在 moonfield-ecs 建一套平行的 `ReadOnlySystemParam` 层级，而当前只有一个调用方；`Param` 从 `&World` 取，各命令读的资源本来就全是只读。有命令需要可变时再议。
- **经 rhi `id()` 访问器的逐 item 管线身份。** 落选：为一个去重键重开 rhi API；地址身份是 `TrackedRenderPass` 的私有细节且逐 pass 重置。
- **被取代 note 的 `PipelineCache`。** 推迟而非放弃：注册表已泛化到多命令，但今天只有一条 graphics 管线（`Core3dPipeline`，在 `PrepareViews` 按着色器版本重建）；带键缓存只会有一条记录，键类型纯属臆测。第一个真实客户——GS compute 内核或多材质——会带着具体键一起到来。
- **被取代 note 在 draw 对象上的 `prepare` 钩子。** 放弃：`RenderCommandState` 里的 `SystemState` 已携带每命令的持久状态；钩子没有调用方。

## Consequences

- phase item 携带 `DrawFunctionId<Opaque3d>`；queue 系统经注册表解析 id，不再经 newtype 资源。
- draw 命令是实现 `RenderCommand` 的单元结构——无状态、可组合、无需注册表即可测试（注册表测试钉死类型化查找与索引 dispatch）。
- 不挂 `RenderPlugin` 的 feature（测试）没有 set 锚，排序退化为注册序——queue/sort 测试按真实 app 的组合挂 `RenderPlugin`。

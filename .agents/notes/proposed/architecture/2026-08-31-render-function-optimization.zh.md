# Agent Note: Render-function pattern optimization (SystemParam dispatch, pipeline cache, tracked pass)

Status: proposed

[English](2026-08-31-render-function-optimization.md)

## Problem

render-function 机制——`moonfield-render-core/src/render_phase.rs` 中的 `PhaseItem` / `DrawFunction<P>` / `DrawFunctions<P>` / `RenderPhase<P>`，在 `moonfield-render-feature/src/render_phase.rs` 中以 `Opaque3d` + `DrawMesh` 唯一实例化（见 [render-phase 框架](../../implemented/architecture/2026-08-26-render-phase-framework.md)）——存在每帧开销，且阻碍多材质使用：

- `DrawFunctions<P>` 用 `HashMap<u32, Box<dyn DrawFunction<P>>>` 存放 draw function；dispatch 循环里每个 phase item 每帧付出一次哈希查找。
- `DrawMesh::draw` 每个 item 每帧执行三次 `world.get_resource` 与两次 `HashMap` 查找，均未提到 item 循环之外。
- 每个 item 重复绑定图形管线、顶点缓冲、索引缓冲与 push constants，无去重；共享管线的 item 重复发出相同的绑定调用。
- `Opaque3d` 不携带 pipeline 字段；`DrawMesh` 获取单一共享 `Core3dPipeline` 资源，一个 opaque phase 无法容纳多于一种材质。
- `DrawFunctionId(u32)` 不携带 `PhaseItem` 类型参数；`DrawFunctions<Opaque3d>` 铸造的 id 传给另一 phase 的 registry 仍能编译，且经手写 `Opaque3dDrawFunction` newtype 资源传给 queue 系统。
- `main_opaque_pass_3d` 每帧克隆整个 `Core3dFrame`（全部 view 及其 phase `Vec`），以规避与 `WindowSurfaces` 的借用冲突。
- `view.opaque.sort()` 在 queue 系统内部调用，“pass 运行时 items 已排序”没有唯一所有者。
- `Core3dPipeline` 在某个 `Render` 系统内经 `contains_resource` 守卫惰性创建，带有副作用。

该机制对 `P` 泛型，但仅由一个 phase item 与一个 draw function 实例化。目标规模是少量材质加上 alpha 与 transparent 阶段；带 GPU 剔除 multidraw 的完整 PBR 不在范围内。

## Proposal

在 moonfield-ecs 已提供前置 `SystemParam` 机制之处，将 dispatch 路径对齐 Bevy 的 `RenderCommand` 模型，但不采纳 Bevy 的全部表面。工作按依赖分阶段；每阶段结束时工作区可构建且测试通过。

### Dispatch 存储与 id 安全

`DrawFunctions<P>` 改为 `Vec<Box<dyn DrawFunction<P>>>`，以 `DrawFunctionId` 索引；查找为 `O(1)`，无哈希。`DrawFunctionId<P>` 携带 `PhantomData<P>`，id 在编译期绑定到其 phase。`DrawFunctions<P>::id::<T>()` 经 `TypeId` 取回 id，移除手写的 `Opaque3dDrawFunction` newtype 资源。

### SystemParam 驱动的 render command

无状态 `RenderCommand<P>` trait 取代有状态的 `DrawFunction<P>::draw(&self, …)`。它声明 `type Param: SystemParam` 与 `fn render(world, item, pass, param)`。moonfield-ecs 新增 `SystemState<P>`——把 `FunctionSystem` 已用的 `init_state` + `fetch` 配对提升为可复用容器——缓存 param 状态。`RenderCommandState<P, C>` 包装 `SystemState<C::Param>`，实现 object-safe 的 `Draw<P>`，以 `Box<dyn Draw<P>>` 存储。draw 时 `param = state.get(world)` 一次取齐全部资源；逐 item 的 `get_resource` 与 `HashMap` 调用消失。`Param` 约束为只读访问，render command 无法修改 render world。保留空默认 `fn prepare(&mut self, world)` 钩子供未来逐 phase 预热。

### 管线缓存与逐 item 管线

`RenderPipelineCache` 以 (shader, 静态状态) 键映射到 `CachedRenderPipelineId`。`PhaseItem` 新增 `cached_pipeline()` 作为可选 trait；`Opaque3d` 携带 pipeline id。`DrawMesh` 从 `item.cached_pipeline()` 绑定管线，而非单例资源；queue 系统按材质分配 id。一个 opaque phase 随之容纳多种材质。

### 跟踪式 render pass

moonfield-render-core 新增 `TrackedRenderPass`，包装 `&CommandBuffer` 加一份 `DrawState` 缓存，记录当前管线、顶点缓冲、索引缓冲与 bind group。每个 `set_*` 在状态未变时短路；render pass 开始时 `reset_tracking()` 使缓存失效。draw function 接收 `&mut TrackedRenderPass` 而非 `&CommandBuffer`，共享管线的相邻 item 间重复绑定被消除。

### 排序归属与管线创建

新增 `PhaseSort` schedule 步骤，由它排序全部 phase；queue 系统只负责 add item。`Core3dPipeline` 构造移至 `RenderPrepare`。`main_opaque_pass_3d` 不再克隆 `Core3dFrame` 即可读取。

## Acceptance criteria

- [ ] `DrawFunctions<P>` 存为 `Vec<Box<dyn DrawFunction<P>>>`；`get(id)` 以 `DrawFunctionId` 索引，无 `HashMap`。
- [ ] `DrawFunctionId<P>` 按其 phase 参数化；`DrawFunctions<P>::id::<T>()` 经 `TypeId` 返回 id；移除 `Opaque3dDrawFunction` newtype 资源。
- [ ] 新增 `PhaseSort` schedule 步骤排序全部 phase；无 queue 系统调用 `sort()`。
- [ ] `Core3dPipeline` 在 `RenderPrepare` 构造；无 `Render` 系统经 `contains_resource` 守卫创建它。
- [ ] `main_opaque_pass_3d` 不克隆 `Core3dFrame` 即可渲染。
- [ ] moonfield-ecs 存在 `SystemState<P>`，含 `new(world)` 与 `get(world)`；`RenderCommand<P>` 声明 `type Param: SystemParam`（只读）与 `fn render(world, item, pass, param)`；`DrawMesh` 无状态。
- [ ] `RenderPipelineCache` 以 (shader, state) 键映射到 `CachedRenderPipelineId`；`Opaque3d` 暴露 `cached_pipeline()`；`DrawMesh` 绑定 `item.cached_pipeline()`。
- [ ] `TrackedRenderPass` 去重管线、顶点缓冲、索引缓冲与 bind group 绑定；draw function 接收 `&mut TrackedRenderPass`。
- [ ] `cargo fmt`、`cargo clippy --workspace --all-targets`、`cargo test --workspace` 与 `python3 scripts/verify_agents.py` 通过。

## Risks

- `RenderCommandState::draw` 在持有 `&mut self` 时从 `&World` 取 `Param`；moonfield render 系统已持有 `&mut World`，借用可行，但 Bevy 因 `draw` 为 `&mut self` 而对 `DrawFunctions` 取的逐 phase 写锁必须复刻，否则 draw 内的 `&World` 资源读取与 registry 借用冲突。
- `SystemState<P>` 是 moonfield-ecs 的新公共表面；将 `RenderCommand::Param` 约束为只读，保住“render schedule 不在 draw 调用内修改 render world”这一性质。
- `RenderPipelineCache` 是新子系统；过期或缺失的缓存键产生错误或缺失的管线，无驱逐时缓存随 (shader, state) 组合数增长。
- `TrackedRenderPass` 去重仅在缓存状态与活体 Vulkan 状态一致时正确；render pass 开始或 command buffer 重置时必须运行 `reset_tracking()`，否则被跳过的绑定留下陈旧状态。
- 将排序移至 `PhaseSort` 要求每个 queue 系统排在它之前；错序的 queue 系统会静默产出无序渲染。
- 移除 `Core3dFrame` 克隆需重排 `main_opaque_pass_3d` 对 `WindowSurfaces` 与 frame 的借用；错误的重构会把克隆换成借用检查失败或陈旧 frame 读取。

## Alternatives considered

- **无状态 `RenderCommand` 加元组组合（Bevy 的 `SetItemPipeline` / `SetMeshBindGroup` / `DrawMesh` 拆分）**：本方案否决。Bevy 的组合依赖 bind-group 缓存、管线缓存与多材质 bind-group 系统，moonfield 的 RHI 尚无这些基础设施；在基础设施就位前把 `DrawMesh` 拆成 `SetXxx` 是为组合而组合。待 bind-group 缓存落地后再议。
- **sorted-merge batching 与 instancing**：有条件。batching 需把逐实例数据从 push constants 移入 instance buffer，是更大的 RHI 改动，且仅在同一 mesh 被大量实例化时获益。除非场景在多个 entity 间复用 mesh，否则暂缓；在此之前每个 item 保持一次 `draw_indexed(.., 1, ..)`。
- **带 multi-draw indirect 的 binned phase（Bevy 的 `BinnedRenderPhase`）**：本规模否决。binning 与 GPU 剔除 multidraw 在数千次 draw 的完整 PBR 规模下才划算；在少量材质与阶段下，它们只增加记账负担（`BinKey` / `BatchSetKey` / 三桶存储）而无可见收益。
- **Bevy 的 `ViewQuery` / `ItemQuery` on `RenderCommand`**：否决。moonfield 的 pass 已设置 viewport、depth 与 cull 状态，`DrawMesh` 只读取 item；moonfield-ecs 的 `Query` 不携带逐实体缓存状态（`State = ()`），view 与 item 查询参数只增加无支撑机制的概念。仅保留 `Param`。
- **独立的逐帧 `prepare` 优化**：并入 `SystemState`。`SystemState::get` 已消除逐 item 资源获取；`prepare` 钩子仅作为未来预热的空默认保留，不作为独立改动。
- **保留 `HashMap` dispatch 与逐 item 资源获取**：否决。逐 item 哈希查找与三次 `get_resource` 是每帧浪费，相比索引 `Vec` 与缓存 param 状态无任何收益。

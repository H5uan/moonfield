# Agent Note: ECS-driven infrastructure roadmap

Status: implemented

[English](2026-08-19-ecs-driven-infrastructure-roadmap.md)

## Problem

引擎基础设施此前是临时拼凑的:app 运行的是无排序的扁平 system 向量,编辑器拥有整个渲染器,场景中没有任何可检查的东西——没有层级、没有时间、没有资产,编辑器无法查看或编辑游戏可见世界的内容。项目需要自己的 ECS 架构路线(而非第三方依赖),同时需要一个能检查并编辑引擎所暴露一切的编辑器。

## Decision

以主流保留模式 ECS 的风格,让所有引擎基础设施经由 ECS 驱动,双重目标是:(a) 拥有我们自己的 ECS 架构路线;(b) 让一切都能被编辑器检查/编辑。范围是**中间层**:游戏可见表面(场景实体、Transform 层级、相机、时间)+ 资产系统 + 渲染资源的 ECS 镜像。明确排除:独立的渲染世界 / extract 分离——那属于未来的多线程渲染工作。

方法:从参考实现的本地检出移植运行时机制,在架构层面借鉴,而非镜像其 API。不使用 proc-macro crate,唯一获批的例外是 `moonfield-reflect-derive`;`Component`/`Resource` 保持 blanket impl;system 参数使用手写 impl 加元组宏;schedule 只保留单线程执行器。由此产生的运行时机制由 [docs/architecture.md](../../../../docs/architecture.md) 承载。

路线图以八个里程碑落地:

1. **ECS 核心** —— system 参数(`Res`/`ResMut`/`Query`/`Local`/`Commands`)、带 `before`/`after` 排序的命名 schedule(稳定拓扑排序,单线程)、每个 system 运行后排空 commands;`App` 驱动 `Startup`/`Update`/`Render`/`Shutdown`,`AppExit` resource 取代旧的 `-> bool` 退出约定。
2. **组件钩子** —— 每种组件类型的 `on_add`/`on_insert`/`on_discard`/`on_remove`(外加 `on_despawn`);discard 在结构变更之前触发,其余在之后;运行中的钩子会被移出注册表,以防止同钩子递归。
3. **关系与层级** —— 由钩子保持同步的通用 `Relationship`/`RelationshipTarget`;`ChildOf`/`Children` 具备 linked-spawn 递归 despawn,插入成环会 panic;`Transform`/`GlobalTransform` 位于 `moonfield-math`,由 `HierarchyPlugin` 在 `Update` 中的 system 传播。
4. **时间** —— `moonfield-time` 中的 `Time<Real>`/`Time<Virtual>`/通用 `Time`(虚拟时钟支持暂停、相对速度、`max_delta` 钳制);后端每帧在 `App::update` 之前推进时钟一次,缺失的时钟会被惰性插入。
5. **渲染接缝** —— `RenderPlugin` 创建 Vulkan 实例与设备,并作为共享的 `RenderDevice` world resource 插入(容忍无头环境);`WindowRenderer`/`EditorState` 只保留窗口绑定和编辑器专有的对象;渲染阶段 system 直接查询 `World`,没有 extract 层。
6. **场景面板切片** —— 编辑器视口把 ECS 场景(`moonfield-render::scene` 中的 `Camera`/`PrimaryCamera`/`MeshRenderer`)渲染进离屏目标;Hierarchy 面板(来自 `ChildOf`/`Children` 的实体树、`Name` 标签、选择)与 Inspector 面板(由 `InspectorRegistry` 自动生成);`MOONFIELD_EDITOR_AUTO_CLOSE` 冒烟测试。
7. **迷你反射** —— `Reflect` trait(命名字段枚举、动态读写、叶子通过 `Any` 向下转换)加 `moonfield-reflect-derive` 中的 `#[derive(Reflect)]`;Inspector 对其通用遍历。没有 `DynamicStruct`、类型注册表或序列化。
8. **资产,同步优先** —— 零依赖的 `Assets<T>` slot-map 存储与 index+generation 的 `Handle<T>`;`SplatCloud` 是第一个资产,由调用方同步加载;训练状态留在 `World` 之外。

## Alternatives considered

- **依赖完整的第三方 ECS。** 拒绝:双重目标包含拥有自己的架构路线——钩子、关系与 schedule 是编辑器和未来渲染分离赖以构建的接缝,第三方核心会把这些接缝变成别人的 API。它还会引入本工作区刻意回避的 proc-macro 密集型栈。
- **现在就做渲染世界分离。** 拒绝(推迟):没有多线程渲染就没有可 extract 的对象,双世界分离会让每种场景类型翻倍却毫无当前收益。单线程渲染接缝——渲染阶段 system 直接查询 `World`——让日后的分离保持可能。
- **在 API 层面镜像参考实现。** 拒绝:镜像其 API 表面会把它的复杂度预算整体引入(derive、变更检测、类型注册表)。架构层面的借鉴取其语义——schedule、钩子、关系——而让表面保持本工作区的原生风格。

## Consequences

- 一切游戏可见的东西现在都是编辑器可检查、可编辑的 ECS 数据:Hierarchy 面板显示实时实体树,Inspector 可编辑任何已注册组件,时间/相机/资产都是普通的 resource 与 component。
- 渲染器不再属于编辑器:共享的 `RenderDevice` resource 同时服务游戏与编辑器路径,无头运行时退化为无设备而非 panic。
- 依赖方向在构造上保持无环:数学类型不认识 ECS,渲染器 crate 不认识 ECS,反射位于两者之下(`moonfield-reflect` 直接依赖 glam,以避免 math↔reflect 循环)。
- 已知欠债(已记录,未排期):渲染世界/extract 分离、异步 `AssetServer` 加任务池、编辑器视口中的真实 splat 光栅化、多线程执行器、完整 observer、完整反射(`DynamicStruct`)、强/弱句柄引用计数,以及音频/物理/序列化/网络层。

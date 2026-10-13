# Agent Note: BSN-style scene templates with a glTF carrier

Status: implemented

[English](2026-08-21-bsn-style-scene-templates.md)

## Problem

工作区此前没有场景保存/加载：hierarchy、`Transform`、`Camera` 与各组件
只存在于 `World` 中，没有任何序列化通道；资产加载由调用方同步完成且
不去重——编辑器直接解析 PLY 文件，每次加载都重新解析一遍。
[跟随参考实现的 roadmap](2026-08-19-ecs-driven-infrastructure-roadmap.zh.md)
把这一层指向 vendored 的 0.20-dev 源码树（`target/bevy-src`），其 BSN
方向用类型化 template 和两阶段的 scene→resolved→apply 管线取代了已删除
的 `DynamicScene`/RON 体系。但 0.20 还没有运行时文本格式，也没有保存
方向，因此即便跟随 BSN，文件格式层仍须自建。

## Decision

本系统是 BSN 的同步迷你版——类型化 template、两阶段 apply、零运行时
反射——上面叠加自有的 glTF 2.0 JSON 载体。机制描述见
[docs/architecture.md](../../../../docs/architecture.md)。

- `moonfield-ecs` 获得类型化的一半：`Template` trait（纯数据，在
  `TemplateContext { world: &mut World }` 中构建其 `Output`）、让一切
  `Clone` 类型成为自身 template 的 blanket impl，以及 `TemplateError`；
  `World::iter_entities()` 为保存侧枚举实体。
- `moonfield-asset` 获得同步的 `AssetServer` world resource：
  `AssetLoader` 实现（`Send + Sync`，因为 blanket `Resource` impl 有此
  要求）按文件扩展名分发；`(TypeId, PathBuf)` 路径缓存为重复加载提供
  命中，缓存槽失效时重新加载。该 crate 保持零依赖、无异步。
- 新 crate `moonfield-scene` 承载场景的一半：`HandleTemplate<T>`（一个
  路径或一个已解析的 handle）、类型擦除的 `SceneTemplate`（对输出为
  `Component` 的 `Template` 做 blanket impl）、`ResolvedScene`（一个实体
  的 template 集合加其子树；`apply` 负责 spawn 并用 `ChildOf` 链接），
  以及 `SceneRegistry`——稳定的短名字（`"transform"`，绝不用 Rust 类型
  路径）映射到 glTF 原生条目（transform/camera/hierarchy/name）或 extras
  通道条目（泛型 serde 或自定义钩子）。`save_scene`/`load_scene` 借助
  `gltf-json` 把 world 映射为 glTF 2.0 JSON 文档；`SceneError` 归拢各类
  失败。
- `moonfield-render` 只给 `scene::MeshRenderer` 加 serde derive——目前
  唯一以纯数据形式走 extras 通道的组件。
- 编辑器完成接线：`SplatCloudLoader`、`editor_asset_server()` /
  `editor_scene_registry()` 两个 resource、改经 AssetServer 的
  `load_splat_cloud`（按路径去重），以及 Hierarchy 面板里的 Scene 路径
  输入框和 Save/Load 按钮（`SceneIoState`）。

对照 vendored 的 0.20-dev 源码，有意跳过：`bsn!` proc-macro DSL、
`ScenePatch` 缓存、`QueuedScenes`/`WaitingScenes`（仅异步）、
`BundleWriter` bump arena、命名实体引用，以及完整的 glTF mesh/material
导入。

## Alternatives considered

- **移植旧的 `DynamicScene`/RON 反射体系。** 否决：它在上游已被删除——
  我们跟随的 0.20-dev 源码树早已越过它——而且它需要运行时反射（类型
  注册表、`DynamicStruct`），这正是本工作区有意不设的东西；
  `moonfield-reflect` 里的迷你反射只服务于编辑器检视。
- **在类型化 template 之上用 RON 作文本载体。** 否决：RON 同样需要逐
  组件的 serde 接线，而在交换能力上相对 JSON 毫无增益——没有任何 DCC
  或外部工具能读它。
- **USD。** 否决，因体量不成比例：带 layer、reference、variant 的组合
  引擎远超场景存取所需，其依赖体量也压过这个迷你版。
- **自定义 schema 的纯 JSON。** 否决其载体资格：serde 成本相同，但
  hierarchy/TRS/camera 映射将是我们自创的发明，毫无互操作性。glTF 原生
  覆盖这三者，DCC 互通与未来的 mesh 导入免费获得，而 `serde_json` 仍在
  `node.extras` 内充当 extras 通道的编码。

## Consequences

- 文件格式即 `SceneRegistry` 的公开契约：稳定的短名字，绝不用 Rust 类型
  路径，组件改名不会破坏既有文件。未知的 `extras.components` 键在加载时
  跳过而非报错，因此由更新的 registry 写出的场景仍可加载。
- 产出是合法的 glTF 2.0 文档，外部 DCC 可直接打开。有损边界是显式的：
  matrix 形式的节点加载后没有 `Transform`，正交相机加载后没有 `Camera`，
  `Camera::clear_color` 因 glTF 无对应字段而放在 `extras.camera`。
- handle 组件在文件中是纯路径字符串；加载场景时经 `AssetServer` 解析，
  因此在同一 world 中重载会复用缓存的资产槽，而不是重新解析。
- `GlobalTransform` 从不注册也不保存——加载后由 hierarchy 传播系统重新
  计算。
- 一切都在调用线程上阻塞：无异步队列、无热重载、无后台加载。对今天的
  编辑器合适；场景变大后是已知欠账。

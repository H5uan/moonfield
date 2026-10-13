# Agent Note: Scene save/load drop per-entity extras rebuild and double JSON parse

Status: implemented

[English](2026-09-19-scene-save-load-typed-parse-and-extras-cache.md)

## Problem

moonfield-scene 的 save/load 路径上有两处浪费：

- `save_scene` 对每个实体调用一次 `SceneRegistry::extras_entries`
  （在 `has_registered_component` 内），每个节点再调一次（在
  `save_node` 内，随递归成倍放大）。每次调用都为 extras 通道条目分配
  一个 `Vec` 并重新排序——注册表 `HashMap` 的迭代顺序在每次存档中被
  重复归一化成千上万次。
- `parse_root` 先把整个文档解析成 `serde_json::Value`，给缺少 `nodes`
  键的 scene 对象补上 `"nodes": []`，再把 `Value` 转成
  `gltf_json::Root`——每个场景文件被完整解析两次，其存在的唯一原因
  是 gltf-json 的 `Scene::nodes` 只有 `skip_serializing_if` 而没有
  `#[serde(default)]`，导致 `save_scene` 写出的空场景
  （`"scenes": [{}]`）无法直接解析成 `Root`。

## Decision

- `save_scene` 只收集一次 `registry.extras_entries()`，把
  `&[(&str, SaveFn)]` 切片下传给 `has_registered_component` 和
  `save_node`；逐实体检查变成无分配的切片扫描。缓存按存档调用存在而
  不放进注册表：`SceneRegistry` 是带公开 `&mut self` 注册方法的世界
  资源，内部缓存需要在每次变更时失效，换不来额外收益。
- `parse_root` 改为经 `RootFile` 的单次类型化解析：`RootFile` 是
  `gltf_json::Root` 的加载侧镜像，其 `scenes` 为 `SceneFile`——
  `gltf_json::Scene` 的镜像，在 `nodes` 上带 `#[serde(default)]`。
  镜像逐字段手写而不是用 `#[serde(flatten)]` 包裹 `Root`：flatten
  会把文档经 serde 私有 `Content` 缓冲，而它会拒绝 gltf-json 存在
  节点上的 `Box<RawValue>` extras。

## Alternatives considered

- **在 `SceneRegistry` 内缓存排序后的 extras 列表**（如用
  `OnceCell`，由 `register*` 方法失效）。`save_scene` 接收
  `&SceneRegistry`，这需要内部可变性加失效纪律；每次存档构建一个
  `Vec` 已经把 O（条目数） 从每实体一次降到每存档一次，消除了真正的
  浪费。
- **用 `#[serde(flatten)]` 的 `RootFile { scenes, root: Root }`
  包装。** 运行时失败：serde 的 flatten 缓冲不支持
  `serde_json::value::RawValue`，而 gltf-json 的 `Extras` 就是
  `Option<Box<RawValue>>`，任何节点 extras 都会让解析报错。
- **先试 `Root::from_str`，失败再回退到 `Value` 补丁路径。**
  保留两条代码路径，且让空场景回环——正是 `save_scene` 自己产出的
  情形——永远走慢路径。

## Consequences

- 存档每次调用只收集一次 `extras_entries`；排序顺序（确定性文档
  输出）不变。
- 加载只解析文档一次。未知的 `extras.components` 键仍被跳过，会被
  gltf-json 拒绝的文档仍以 `SceneError::Json` 失败。
- `RootFile`/`SceneFile` 必须跟随 gltf-json 的 `Root`/`Scene`
  字段表；该依赖由 workspace 锁定（`gltf-json = "1.4"`），上游新增
  字段会在镜像更新前于加载时静默丢弃——这与 gltf-json 自身不带
  `deny_unknown_fields` 的结构体对未知键的处理是同样的取舍。
- `moonfield-scene` 对 `moonfield-render-feature` 的 dev-dependency
  启用 `mesh` feature：该 crate 在 `default-features = false` 下无法
  编译（其 plugin 模块无条件导入 mesh 门控的模块），因此单独运行
  `cargo test -p moonfield-scene` 依赖 dev-dependency 声明其测试代码
  所用的 feature。

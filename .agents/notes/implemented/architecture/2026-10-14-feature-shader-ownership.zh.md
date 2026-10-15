# Agent Note: Feature shader ownership and composition validation

Status: implemented

[English](2026-10-14-feature-shader-ownership.md)

## Problem

[架构契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
的第四项：`EditorPlugin` 负责加载并注册所有 pipeline 的 shader 请求（`core_3d`、splat
三件套和 egui 一视同仁），因此脱离编辑器做渲染就意味着复刻编辑器的初始化流程。并且
`RenderFeaturePlugin` 用裸的 `insert_resource` 默认值插入它的 asset 存储，会静默替换应用
预先配置好的存储。

## Decision

**feature 自行声明并加载自己的 shader。** `load_feature_shaders`（在 render-feature 的
shader 模块）是共享的启动路径：应用已提供 `AssetServer` 和 `Assets<Shader>` 时复用（否则
创建一个只有 `.slang` 加载器的最小 server 和一个空存储），从 `<assets>/shaders/` 加载各
feature shader 并推送 pipeline 请求。`RenderFeaturePlugin` 为 `core_3d.slang` 注册一个
`Startup` 系统；`SplatForwardPlugin` 为 `radix_sort.slang`、`gaussian.slang` 和
`gs/composite.slang` 注册一个。编辑器只保留一个属于自己的请求：`egui.slang`
（`load_editor_shaders`）。

**插件初始化不替换既有配置。** `World` 和 `App` 新增 `init_resource`（仅在缺失时插入默认
值）；两个 feature 插件都用它初始化 asset 存储、请求列表和预编译产物资源，应用提供的存
储因此在组合后存活。

**组合完成后校验。** `RenderFeaturePlugin::finish` 在 render world 的 `Render` 调度缺少
`PrepareAssets`/`Queue`/`PrepareViews`/`CameraDriver` 锚点（即 `RenderPlugin` 缺失）时带
着补救方法 panic；`SplatForwardPlugin::finish` 对 `Core3d` 调度的 `Core3dOpaquePass` 锚点
（即 `RenderFeaturePlugin` 缺失）做同样的事。两者背后是新增的 `Schedule::has_set` 查询。

**显式请求优先于默认值。** 每个
`FeatureShaderSpec` 在文件和请求构建函数之前声明 pipeline 名称。Startup 根据该名称
检查，仅加载缺失请求；应用的 handle、入口、能力和反射选择均被保留，且不读取默认文件。
`PipelineShaders::push` 保持显式替换接口。

## Alternatives considered

- **在插件 build 时加载 feature shader。** 未采用：应用的 `AssetServer` 常常在插件 build
  之后才插入（编辑器在 `EditorPlugin::build` 期间才插入自己的 server），build 时加载会造
  出一个互相竞争的 server，而不是复用配置好的那个。`Startup` 在所有插件 build 完成后运行，
  且仍早于第一次提取。
- **锚点缺失时告警而不是 panic。** 未采用：该失效模式是系统在没有锚点的情况下按注册顺序
  运行——GPU 工作的顺序被静默打乱。`finish` 时的 panic 指名缺失的插件和修法；告警只会被
  当作噪音。
- **shader 路径配置资源。** 暂缓：shader 目录已经经由 `moonfield_asset::assets_dir` 解析
  （含 `MOONFIELD_ASSETS_DIR` 覆盖），应用也可以在 startup 之前自行推送请求。目前还没有
  需要逐 pipeline 配置路径的消费者。

## Consequences

- 最小应用（`RenderPlugin` + `RenderFeaturePlugin`，可选 `SplatForwardPlugin`）无需编辑器
  即可渲染 mesh 和 splat；编辑器的职责收窄为 egui 与场景 I/O。
- `insert_resource` 保持替换语义；`init_resource` 是组合安全的形式，插件作者按资源选择。
- 测试钉住该契约：render-feature 插件测试覆盖存储保留与启动时 shader 自注册，锚点 panic
  以 `should_panic` 测试覆盖，`moonfield-editor` 的测试收窄为只加载 egui。

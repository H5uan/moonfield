# Agent Note: Shaders as assets with render-world prepared compilation

Status: implemented

[English](2026-09-06-shader-as-asset.md)

## Problem

生产 shader 已经是 `assets/shaders/` 下的文件(见
[Shaders sourced from assets/shaders files](2026-08-26-shader-sourcing-from-files.zh.md)),
但每条管线仍自行定位并编译:`Core3dPipeline` 与编辑器的 `EguiPipeline` 通过
`env!("CARGO_MANIFEST_DIR")` 拼接 `../../assets/shaders` 解析目录——编译期固化、
不可重定位——并直接驱动 RHI 的 `Compiler`。shader 是唯一游离在 asset 层之外的渲染
输入:没有 `Handle`、没有 `AssetRevision`、没有 extraction,下游无法感知 shader 变化,
且路径解析属于渲染器内部而非应用。

## Decision

新 crate `moonfield-shader` 拥有 `Shader` 资产——源文件路径加上 Slang 源码文本——以及
通过 `AssetServer` 服务 `.slang` 文件的 `SlangLoader`(同步 `std::fs` 读取;asset 层按
设计仅同步)。资产不携带入口点元数据:入口点在编译期由 Slang 反射发现。编译、反射与
root binding 仍留在 `moonfield-rhi`。

渲染侧在 `moonfield-render-feature::shader` 中镜像 mesh 特性的 prepared-asset 模式:

- 管线以 `PipelineShader` 请求声明其 shader 需求——资产句柄、带 capabilities 的入口点
  列表、以及用于 root binding 反射的入口——由加载 shader 资产的一方注册进主世界的
  `PipelineShaders` 资源(编辑器在启动时通过其 `AssetServer` 加载 `core_3d.slang` 与
  `egui.slang`)。
- `extract_shader_assets` 把被请求的 shader(按 revision 匹配,同 `extract_mesh_assets`)
  与请求列表复制进 render world。
- `prepare_shaders`(`RenderPrepare`)把 `AssetRevision` 前进过的请求编译进 render
  world 的 `PreparedShaders` 资源——它拥有共享的 `ShaderCache`(惰性创建:首次编译前
  不需要 Slang 会话或 Vulkan 设备)。编译基于资产的源码文本,因此缓存的 memoization 键
  能感知源码变化。`PreparedShaders` 以管线名为键,因为编译的入口集合由管线声明;每个
  槽位记录 prepared 产物(`PreparedShader`:模块反射加上每个入口一个
  `CompiledShader`)或编译错误,因此损坏的源码不会每帧重编译,pass 继续运行已构建的
  管线。
- `Core3dPipeline` 与 `EguiPipeline` 持有各自的 `Handle<Shader>` 及构建时的 revision;
  pass 在 prepared revision 前进时重建管线,在 shader 未就绪时以一次性日志跳过。

两处管线内的 `CARGO_MANIFEST_DIR` shader 查找已删除。路径解析移交给应用接线:编辑器
的启动加载把仓库 shader 目录传给 `AssetServer::load`,沿用默认场景 mesh 的约定。RHI
新增了一个纯增量方法 `ShaderCache::compile_source_reflection`——`compile_file_reflection`
的源码文本对应物(带 memoization)——使 prepared shader 从资产源码编译,而不是重读文件
(基于文件的缓存键无法感知同路径下的源码变化)。

## Alternatives considered

- **shader 保持代码内部。** 拒绝:这正是本笔记替换的现状——渲染器内部的编译期固化
  路径、没有 revision 跟踪、asset 层无法感知 shader。
- **bevy_shader 式完整 asset-graph crate**(带 import 解析与热重载)。拒绝:asset 层
  刻意仅同步、无文件监视,且现有 shader 均未使用 `import`;这套机制属于投机。`Shader`
  资产为它留出了空间。
- **经现有 `ShaderCache::compile_file*` 从资产文件路径编译 prepared shader。** 拒绝:
  基于文件的缓存以路径为键,同路径下被编辑或重载的源码会静默复用陈旧产物;以源码文本
  为编译键才让 revision 模型成立。

## Consequences

- 谁拥有应用,谁加载管线 shader;不带编辑器的应用必须自行加载,否则 pass 以一次性日志
  跳过。编辑器在启动时加载,因此首帧行为与原先的构建期编译模型一致。
- `PreparedShaders` 以管线名而非单独的 `AssetId` 为键:两个管线共享一个 shader 资产时
  各自占一个槽位(底层 `ShaderCache` 仍 memoize 相同的编译)。
- shader 编译从管线创建(`Render` 中)移到 `RenderPrepare`——同一帧内提前一个
  schedule,仍在同一线程。
- 重编译失败会把错误记录到新 revision;正在运行的管线不受影响。修复源码并重载资产
  (新 revision)会重新触发编译——仍然没有热重载。

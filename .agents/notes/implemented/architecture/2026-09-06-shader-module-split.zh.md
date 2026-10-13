# Agent Note: Split vulkan/shader.rs into a shader module directory

Status: implemented

[English](2026-09-06-shader-module-split.md)

## Problem

`crates/moonfield-rhi/src/vulkan/shader.rs` 已膨胀到 1479 行,混合了三种职责——
Slang→SPIR-V 编译与缓存、Slang 反射查询、descriptor-heap 管线的 root 参数绑定。文件中
最微妙的部分埋在文件中段:`Reflection`,一个自引用结构体,持有 Slang session 与链接后
的 `ComponentType`,同时保存指向它们的裸指针,并带有手写的 `unsafe impl Send/Sync`,
其不变量只记录在 impl 旁边的两行注释里。要找到这份 unsafe 契约必须通读整个文件。

## Decision

该文件变为模块目录 `crates/moonfield-rhi/src/vulkan/shader/`:

- `mod.rs` — 模块文档、三个子模块共享的私有 `map_slang_error` helper,以及 re-export;
  每个 `vulkan::shader::{...}` 路径与之前完全一致,因此 `vulkan/mod.rs` 和下游 crate
  (`moonfield-render-feature`、`moonfield-editor`)无需改动。
- `compile.rs` — `CompiledShader`、SPIR-V 入口点名提取、Slang stage →
  `vk::ShaderStageFlags` 映射、`Compiler`,以及 `ShaderCache`(含 `ShaderCacheKey`、
  私有的 `get_or_reflect` helper 和全部 `compile_*` 方法,包括
  `compile_source_reflection`)。
- `reflection.rs` — `Reflection`、`UserAttributeRef`/`UserAttributeArg`、`Layout`,
  以及字段类型/字节大小 helper。模块文档明确写出 `Reflection` 的不变量:session 与链接后
  的组件类型由 wrapper 持有,其生命周期必然覆盖裸指针;所有访问都经 `&self` 只读进行。
- `root_binder.rs` — `RootParamKind`、`RootParam`、`RootParamPlace`、`RootBinder`,
  外加产出 `RootParam` 列表(供 binder 消费)的 `Reflection::root_parameters` impl 块。
  把这个 impl 放在这里(而非 `reflection.rs`)正是依赖链保持单向的原因:
  `compile` ← `reflection` ← `root_binder`。

`Reflection` 的字段变为 `pub(super)`,以便 `compile.rs` 构造 wrapper、`root_binder.rs`
解引用指针;没有名字越过模块原有的 re-export 边界。测试随其覆盖的类型一起迁移;codegen
测试的 `include_str!` 路径因目录加深一层而多了一个 `../`。

## Alternatives considered

- **所有内容保留在一个文件。** 拒绝:文件已超过能靠浏览发现 unsafe `Reflection` 契约的
  规模;拆分才让这份契约拥有自己的模块文档。
- **把反射与 root binding 移入独立 crate。** 拒绝:这套机制是 rhi 的私有绑定契约——
  `CompiledShader` 的字段是 `pub(crate)`,wrapper 的构造是模块私有的——抽出会迫使
  `scripts/verify_rhi_boundary.py` 守护的公共 API 变宽,而并不存在复用方。
- **把 `Reflection::root_parameters` 留在 `reflection.rs`。** 拒绝:它返回 `RootParam`,
  会让 `reflection` 反向依赖 `root_binder`,破坏预期的单向链;impl 块可以放在消费方文件
  中而不改变公共路径。

## Consequences

- `Reflection` 的不变量现在写在 `reflection.rs` 的模块文档中,紧挨着它所支撑的
  `unsafe impl Send/Sync`。
- `vulkan::shader::Layout` 等 re-export 路径完全不变;唯一的机械性影响是那处
  `include_str!` 深度调整。
- 后续的 shader 侧工作有了明确归属:编译改动去 `compile.rs`,反射查询去
  `reflection.rs`,push-data 绑定去 `root_binder.rs`。

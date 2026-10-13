# Agent Note: Shaders sourced from assets/shaders files

Status: implemented

[English](2026-08-26-shader-sourcing-from-files.md)

## Problem

两个生产用 shader 模块——core 3D 平光网格 pass 与 egui→Vulkan 后端——都以 `&str` 常量内联在 Rust 源码里(`moonfield-render-feature::core_3d` 的 `VERTEX_SHADER` / `FRAGMENT_SHADER`,`moonfield-editor::egui_vk` 的 `SHADER_SOURCE`)。`Compiler::compile_source_to_spirv` 为了绕开 Slang crate 基于文件的 API,每次构建管线都把源码写进临时文件。内联字符串让 shader 无法被编辑器与 diff 审阅,不复用仓库已有的 assets 布局,而且任何一处 shader 微调都强制触发 Rust 重新编译。

## Decision

生产 shader 现在是仓库 assets 目录 `<repo root>/assets/shaders/` 下的 Slang 文件:

- `core_3d.slang` —— core 3D pass,单个模块包含 `vs_main` / `fs_main`。
- `egui.slang` —— egui 后端,单个模块包含 `vs_main` 与 `fs_gamma` / `fs_linear` 两个 fragment 入口。

管线不再自行定位或编译这些文件:两个 shader 都是经由 asset 层加载的 `Shader` 资产,在 render world 中编译为按 revision 匹配的 prepared 产物——见 [Shaders as assets with render-world prepared compilation](2026-09-06-shader-as-asset.md)。编译器的模块名来自文件路径。

`compile_source_to_spirv` 保留在 RHI 中:headless/offscreen 三角形测试、bindless compute 测试与 `headless_triangle` 示例继续内联 shader,让每个测试保持自包含。

## Alternatives considered

- **保留内联字符串。** 拒绝:这正是本笔记要替换的现状——每次构建管线都要临时文件往返,shader 源码对编辑器和 diff 审阅不可见。
- **用 `include_str!` 嵌入文件。** 拒绝:文件无法就地编辑——任何改动仍然需要 Rust 重新编译,而且源码被复制进二进制却没有任何运行时收益。
- **每个 crate 各自的 `assets/shaders/` 目录。** 拒绝:仓库已经把所有仓库管理的资源集中到根目录 `assets/`(`models/`),两个使用方以相同的相对跳数解析同一个共享目录。

## Consequences

- shader 改动就是普通文件编辑:无需 Rust 重编译,diff 展示的是 shader 本身而不是字符串常量包装。
- 编辑器的启动 shader 加载通过编译期固化的路径解析仓库布局 `<repo root>/assets/shaders/`(与默认场景 mesh 的约定一致);管线本身不含路径。
- 测试与示例 shader(RHI 测试、`headless_triangle`)按设计保持内联,每个 GPU 测试的自包含性得以保留。
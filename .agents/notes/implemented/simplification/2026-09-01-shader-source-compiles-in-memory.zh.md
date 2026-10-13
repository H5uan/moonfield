# Agent Note: Shader source compiles in memory

Status: implemented

[English](2026-09-01-shader-source-compiles-in-memory.md)

## Problem

`Compiler::compile_source_to_spirv` 接收的是内存中的 Slang 源码,却要通过临时文件来编译:`Session::load_module` 是 Slang 基于文件的加载入口,所以源码被写入 `std::env::temp_dir()` 下的 `{module_name}.slang`,编译后再删除。两个 source 入口各自维护一份这样的往返,整条路径还依赖一个可写的系统临时目录。

## Decision

`compile_source_to_spirv` 与 `compile_source_to_spirv_with_capabilities` 改用 `Session::load_module_from_source_string` 加载源码,以 `module_name` 注册模块,并用合成的 `{module_name}.slang` 路径作为诊断名。不再写入或删除任何文件。

文件和 source 两个输入共用的编译管线被拆分为 `Compiler::create_session`(为给定 capabilities 创建 SPIR-V session)与 `Compiler::finish_compile`(查找入口点、链接、提取字节码);`compile_file_to_spirv_impl` 复用同一套辅助函数。

## Alternatives considered

- **保留临时文件往返。** 拒绝:`shader-slang-rs` 在锁定的版本上已经提供 `load_module_from_source_string`,这个 workaround 对抗的是一个并不存在的限制。

## Consequences

- 源码着色器编译是纯内存操作;诊断信息以 `{module_name}.slang` 命名模块。
- 两个 source 入口与文件入口共用同一实现,不再各自持有临时文件变体。
- 内存着色器中的相对 `import` 现在相对合成模块路径(即工作目录)解析,而非系统临时目录。当前没有着色器以这种方式使用相对 import。
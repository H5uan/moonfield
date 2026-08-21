# Agent Note: moonfield tracks shader-slang-rs master

Status: implemented

[English](2026-08-21-shader-slang-master-follow.md)

## Problem

运行编辑器在启动时失败,报 `libslang-compiler.so.0.2026.14.1: cannot open
shared object file`。`shader-slang-rs-sys` 的 build script 把 Slang 运行时库拷到了
`target/debug/build/`,而动态加载器不会搜索该目录。其 `copy_runtime_libs_to_profile_dir`
用 `out_dir.ancestors().nth(3)` 计算 profile 目录,这个假设只对旧版 target 布局
(`<profile>/build/<pkg>-<hash>/out`)成立;新版 Cargo 布局
(`<profile>/build/<name>/<hash>/out`)下会向上多算一层。此外,fork 的 master 分支
已经领先于锁定的 commit,包含破坏性 API 变更(`Reflection::find_type_by_name` 现在
返回 `Result<Option<&Type>>`)。

## Decision

- 依赖跟随 `shader-slang-rs` 的 git master 分支(不 pin `rev`)。bug 修复和功能改进
  先落在该 fork 的主线上;moonfield 只跟随主线。
- `vulkan/shader.rs` 中 `find_type_by_name` 的调用点适配新的 `Result<Option<&Type>>`
  契约:先映射错误,再对 `Option` 做必填处理。
- 上游修复(`fix(sys): resolve profile dir for current cargo target layouts`,位于
  fork 的 master)落地后,不再需要手动把 `libslang*` 复制到 profile 目录。

## Alternatives considered

- **用 `rev` 锁定一个专门的修复分支。** 拒绝:主线是唯一事实来源;锁定分支会增加
  同步维护成本,并偏离 fork 的默认分支。
- **保留本地 `path` 依赖。** 拒绝:机器相关的绝对路径会破坏其他人的构建,也不反映
  fork 的真实状态。
- **用 `[patch]` 在本地承载修复。** 拒绝:这会让代码库与 fork 主线分叉,把本应
  上游的修复重复实现一份。

## Consequences

- 在旧版和新版两种 Cargo target 布局下,`cargo run` / `cargo test` 都能不加手动
  配置地加载 Slang 运行时库。
- `vulkan/shader.rs::struct_layout` 现在会透出 `find_type_by_name` 的查找错误,
  而不是把它折叠成笼统的 not-found 消息。
- 依赖随 fork 主线前进;以后上游的 API 变更会在进入时在此消化。
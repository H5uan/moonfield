# Agent Note: validation layer toggled by a Cargo feature

Status: implemented

[English](2026-08-24-vk-validation-via-cargo-feature.md)

## Problem

Khronos 验证层此前由 `MOONFIELD_VK_VALIDATION` 环境变量在运行时开启,开关是隐藏的全局状态:没有任何清单或 API 表面能体现它,发布构建也可能残留验证路径,CI 无法声明式地表达这一选择。

## Decision

`moonfield-render` 增加 `validation` Cargo feature(默认关闭),`crates/moonfield-render/src/vulkan/instance.rs` 在 `#[cfg(feature = "validation")]` 下追加 `VK_LAYER_KHRONOS_validation`。这成为编译期决策:`cargo run --features moonfield-render/validation` 运行编辑器;headless 测试用 `cargo test --features moonfield-render/validation`。feature 只决定是否请求该 layer;Vulkan SDK 仍须在运行时安装,因为 layer 是以共享库形式在实例创建时加载。将来若加 `VK_EXT_debug_utils` messenger,应挂在同一 feature 下。

## Alternatives considered

- **保留 `MOONFIELD_VK_VALIDATION` 环境变量。** 否决:运行时开关,无清单/API 表面,发布构建可能残留,CI 无法声明。
- **`#[cfg(debug_assertions)]` 调试构建自动开启。** 否决:零配置,随引擎惯例,但放弃逐次/逐 CI 控制——debug 构建无法关闭验证,release 构建无法 وتسجيلات无法开启。
- **Cargo feature 之上再叠一层环境变量。** 否决:一个开关有两个旋钮,feature 本身已足够简单,叠加会以第二层的形式重现隐藏状态问题。

## Consequences

- 现在切换验证需要重新编译——开关是编译期、按 profile 的决策。
- release 构建整体编译掉 layer 请求,`cargo run --release` 永远不会意外请求验证。
- 显式且可声明,CI 作业和编辑器能在自身配置中表达;CLI 仍需安装 Vulkan SDK,与 feature 无关。

(File has 20 lines total.)
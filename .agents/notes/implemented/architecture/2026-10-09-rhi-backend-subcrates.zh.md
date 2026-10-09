# Agent Note: RHI backend sub-crates

Status: implemented

[English](2026-10-09-rhi-backend-subcrates.md)

## Problem

`moonfield-rhi` 原本是单 crate，公共 API 就是具体的 Vulkan 实现：`lib.rs` 用
glob 方式 `vulkan::*` 再导出，公共面因此是隐式的，后端模块路径
（`moonfield_rhi::device`、……）也随之泄漏到公共 API。第二个后端（macOS 的
Metal 4 后端）需要把 Vulkan 依赖闭包（`ash`、`gpu-allocator`、
`shader-slang`）与 Metal 后端链接的东西隔离开；而下游 crate
（`moonfield-render-core`、`moonfield-render-feature`、`moonfield-editor`、
`moonfield-ml`）直接消费具体类型（`Device`、`Memory`、`GpuPtr`）——引入运行时
trait 层意味着改写所有这些签名。

## Decision

`moonfield-rhi` 成为门面 crate，后端子 crate 放在它自己的目录之下：

- `crates/moonfield-rhi/src` —— 门面。`src/lib.rs` 按后端 feature 携带显式的
  再导出清单；这些清单就是 RHI 的全部公共面。`error`、`indirect`、`types`
  模块路径也被再导出，因此 `moonfield_rhi::types::WrapMode` 依然可解析。
- `crates/moonfield-rhi/core` —— `moonfield-rhi-core`，后端无关的词汇表
  （`types.rs`、`error.rs`、`indirect.rs`）。`core` 中不允许出现任何后端：
  原先放在各类型旁边的 `to_vk` 方法迁移到 `vulkan/src/formats.rs`，收敛在
  `ToVk` extension trait 之后；结果码转换变成了自由函数 `from_vk`。
- `crates/moonfield-rhi/vulkan` —— `moonfield-rhi-vulkan`，Vulkan 后端：
  全部 `ash`、`ash-window`、`gpu-allocator`、`shader-slang`，以及验证
  `pub(crate)` 内部实现的 GPU 测试。

依赖方向是 `moonfield-rhi` → 后端子 crate → `core`；子 crate 是 workspace
成员，但有意不进 `[workspace.dependencies]` —— 只有 `moonfield-rhi` 引用它们，
以此标记它们是内部实现。一次构建中恰好激活一个后端 feature（今天是
`vulkan`）。workspace 依赖带 `default-features = false`，因此 editor 和每个
用到后端类型的 crate 在自己的 manifest 里显式启用该 feature；门面的
`default = ["vulkan"]` 只覆盖门面 crate 的独立构建。`validation` feature
转发到后端 crate 的 validation layer。

`gpu-allocator` 是跨后端共享的分配底座：它在 `vulkan` 之外还携带 `metal`
后端（基于 `objc2`/`objc2-metal`），因此各后端子 crate 声明自己的 feature
切片 —— Vulkan 后端取 `std` + `vulkan`，顺带把 `d3d12`/`metal` 切片从构建中
剔除 —— 分配器类型则保持不进公共 API。

`scripts/verify_rhi_boundary.py` 扫描门面、`core` 与每个后端子 crate，禁词
集合从 `ash`/`vk::`/`gpu_allocator` 扩充到 `objc2` 与 `MTL*`。

## Alternatives considered

**基于 trait 的共享 API 层**（抽象的 `Device`/`CommandBuffer` trait，各后端
crate 实现它们）可以支持运行时后端选择，是多后端 RHI 的常见形态。它在这里
落败：Metal 后端落地的同一次改动里，所有下游签名都要变成泛型或 `dyn` 擦除；
而仓库里没有任何东西需要运行时选择 —— editor 是唯一的 binary，平台本身决定
后端。门面再导出保住了具体类型的消费模型；以后仍可以在门面内部引入 trait
层，不必再挪代码。

**同级顶层 crate**（`crates/moonfield-rhi-vulkan` 与 `crates/moonfield-rhi`
并列）能给出同样的依赖隔离。它落败是因为 `crates/` 的语义是一个引擎关注点
一个 crate：后端是 RHI 的实现细节，不是引擎级模块；嵌套让 workspace 列表和
rhi 的规则文件（`crates/moonfield-rhi/AGENTS.md`）保持唯一。

**单 crate 内 cfg 门控模块**可以完全避免拆 crate。它落败是因为 manifest 会把
两个后端的依赖闭包混在一起（同一文件里 feature 门控的 `ash` 与 `objc2`
条目），后端之间的隔离靠 `pub(crate)` 纪律而非包边界。子 crate 提供编译期的
依赖隔离，并让 GPU 测试继续在拥有它们的 crate 里测 `pub(crate)` 内部实现。

## Consequences

下游 `use moonfield_rhi::{...}` 导入不变 —— 门面再导出同名类型。glob 泄漏的
模块路径从公共 API 消失；现在可达的一切都在显式清单上。词汇表转换归后端所有
（`ToVk`、`from_vk`），这带来两处小的词汇表调整：`Format::bytes_per_pixel`
改为 public，`CommandBufferUsage` 增加了公开的 `contains`。单独构建一个消费
后端的 crate（`cargo build -p moonfield-render-core`）依然可行，因为这四个
manifest 显式启用了 `vulkan` feature；不需要后端类型的 crate 可以保持
feature-free。Metal 4 后端以 `metal/` 兄弟目录加 `metal` feature 落地，见
[Metal 4 后端提案](../../proposed/architecture/2026-10-09-metal-4-backend.zh.md)。

# Agent Note: RHI-owned interface and private backend modules

Status: implemented

[English](2026-10-10-rhi-interface-and-backend-layering.md)

## Problem

[后端子 crate 决策](2026-10-09-rhi-backend-subcrates.zh.md)
隔离了后端依赖，并让下游继续使用具体的 RHI 类型。门面层选择后端重导出列表，
但没有定义两套实现必须遵守的公共接口。Vulkan 与 Metal 的导出列表不同，
名称一致也不能保证行为或签名一致。

`Memory` 体现了这一差异：在 Vulkan 中它表示分配类别
（`Default`、`Gpu`、`Readback`）；在 Metal 中它拥有一个缓冲区。
`GpuAllocation` 的构造函数、命令池分配方法和 `CommandBuffer::end` 的返回类型也不一致。
上层无法仅通过切换门面层 feature 来切换后端。

后端知识也进入了调用方：render、editor 和 ml 的 manifest 启用了 `vulkan`，
shader 准备代码指定了 `ShaderTarget::Spirv`。在 crate 与模块之间移动实现文件，
无法解决这些契约问题。接口归属与包组织方式需要分别决策。

## Decision

`moonfield-rhi` 拥有公共接口，后端选择留在其实现内部：

- 单个 crate 承载公共接口：词汇模块（`types.rs`、`error.rs`、
  `indirect.rs`、`shader/`——资源描述、内存类别、能力、共享的 Slang 编译器）
  与门面层的显式重导出列表，后者即全部公共表面。
  `scripts/verify_rhi_boundary.py` 机械地禁止后端类型出现在其中。
- 后端是 `src/backend/` 下的私有模块，编译期按平台选择——见
  [私有模块笔记](2026-10-11-rhi-backend-private-modules.zh.md)。
  [Vulkan pivot](2026-10-13-rhi-vulkan-pivot.zh.md) 随后删除了 Metal 一侧，
  留下 Vulkan 作为唯一后端和一份重导出列表。
- 接口钉住的共同契约：内存类别有后端无关的含义（CPU 可见性、回读、所有权、
  GPU 完成）；命令录制、提交、等待与复用有一致的签名与顺序约束；
  `Stage`/`Access` 语义与 root data 快照行为共享，各后端负责原生映射；
  shader 编译目标由选中的后端决定（`shader_target()` 是 `Spirv`）。
- `Device`、`GpuAllocation` 等公共类型在需要适配或共享不变量时包装选中的
  具体后端实现。上层继续使用具体的 RHI 类型；render、editor 和 ml 不必引入
  后端泛型或 trait object。调用方依赖 `moonfield-rhi`，不固定后端——互斥的
  `vulkan`/`metal` cargo feature 已移除。

## Alternatives considered

**在 RHI 自有接口下保留后端子 crate。** 这种布局保留独立编译单元、依赖限制和 crate 内
GPU 测试。它是迁移的起点；如果这些性质足以抵消额外跨 crate 接口的成本，也可以作为最终
布局。当后端始终是内部实现且没有独立调用方时，优先采用私有模块。

**合并 crate，但继续以重导出后端作为接口。** 这种方式减少 manifest，却没有解决语义差异
或上层对后端的了解。仅迁移文件无法达成本提案的目标。

**将 `Device<B>` 和后端 trait 传播到上层。** 静态多态可以约束实现契约，但让每个调用方
接触它，会扩大调用方需要理解的接口。除非调用方确实需要后端泛型，否则将必要的 trait
保留在内部。

**在整个 RHI 中引入运行时 trait object 或枚举分派。** 这种方式支持在同一二进制中选择
多个后端，但增加了分派和跨后端资源一致性问题，而平台选择部署并不要求这一能力。
如果同平台运行时选择成为需求，再重新评估。

## Consequences

- 单一重导出列表即全部公共表面；`verify_rhi_boundary.py` 门禁扫描单一的
  `src/` 树，target 条件与私有模块可见性承担了子 crate 时代由 crate 边界
  强制执行的依赖与隔离纪律。
- 契约测试套件（`tests/contract_*.rs`）钉住任何后端都必须遵守的公共行为；
  后端本地 GPU 测试以私有后端的 `#[cfg(test)]` 模块形式保留。
- 调用方不再点名后端：manifest 只带一个普通的 `moonfield-rhi` 依赖，
  shader target 由 RHI 给出，render、editor、ml 中没有后端特有的导入。
- macOS 在再有后端落地之前不属于支持的目标平台——取舍由 Vulkan pivot 记录；
  新增 D3D12 或运行时回退时重新评估选择方式，同时保持公共资源和命令契约稳定。
- 任何后端改动都会重编译这一个 RHI crate；包列表更短不代表构建更快。

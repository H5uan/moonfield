# Agent Note: RHI-owned interface and private backend modules

Status: proposed

[English](2026-10-10-rhi-interface-and-backend-layering.md)

## Problem

[后端子 crate 决策](../../implemented/architecture/2026-10-09-rhi-backend-subcrates.zh.md)
隔离了后端依赖，并让下游继续使用具体的 RHI 类型。门面层选择后端重导出列表，
但没有定义两套实现必须遵守的公共接口。
[门面层](../../../../crates/moonfield-rhi/src/lib.rs)中 Vulkan 与 Metal 的导出列表不同，
名称一致也不能保证行为或签名一致。

`Memory` 体现了这一差异：在
[Vulkan](../../../../crates/moonfield-rhi/vulkan/src/memory.rs) 中，它表示分配类别
（`Default`、`Gpu`、`Readback`）；在
[Metal](../../../../crates/moonfield-rhi/metal/src/memory.rs) 中，它拥有一个缓冲区。
`GpuAllocation` 的构造函数、命令池分配方法和 `CommandBuffer::end` 的返回类型也不一致。
上层无法仅通过切换门面层 feature 来切换后端。

后端知识也进入了调用方：render、editor 和 ml 的 manifest 启用了 `vulkan`，
[shader 准备代码](../../../../crates/moonfield-render-feature/src/shader.rs)
指定了 `ShaderTarget::Spirv`。在 crate 与模块之间移动实现文件，无法解决这些契约问题。
接口归属与包组织方式需要分别决策。

## Proposal

由 `moonfield-rhi` 定义公共接口，将后端选择保留在其实现内部。公共契约建立后，
优先采用私有后端模块；建立契约期间保留后端子 crate，让接口改动与文件迁移可以分别验证。
在迁移获准并交付之前，本提案不替代已实施的子 crate 决策。

目标模块职责如下：

```text
moonfield-rhi/src/
    lib.rs          显式公共导出
    types.rs        资源描述、内存类别、能力
    device.rs       公共设备接口与初始化
    command.rs      录制与提交契约
    memory.rs       分配与指针契约
    shared/         已证明可复用的后端无关实现
    shader/         编译、反射与缓存
    backend/
        mod.rs      编译期实现选择
        vulkan/     私有 Vulkan 实现与转换
        metal/      私有 Metal 实现与转换
```

`Device`、`GpuAllocation` 等公共类型在需要适配或共享不变量时，包装选中的具体后端实现。
上层继续使用具体的 RHI 类型；render、editor 和 ml 不必引入后端泛型或 trait object。
不要为已经共享的词汇类型增加转发包装，也不要仅为了测试而暴露内部接口。

先定义公共契约，再适配实现：

- 为内存类别和分配对象定义不同且与后端无关的含义。规定 CPU 可见性、回读、所有权及
  GPU 完成要求，不照搬某个后端的存储表示。
- 为命令录制、提交、等待和复用提供一致的签名与顺序约束。当调用方只需要提交完成标记时，
  隐藏队列族选择和原生同步对象。
- 定义共享的 `Stage`/`Access` 语义和 root data 快照行为。各后端负责原生映射。
  不支持的操作必须报错，或要求显式能力检查；公共契约承诺产生效果时，不能静默忽略。
- 从选中的设备或后端确定 shader 编译目标。离线编译工具可以显式指定目标，
  常规渲染初始化不应承担这一选择。
- 用共享能力描述表示可选渲染能力；选中某个后端，不代表所有设备都支持这些能力。

对于 [Metal 提案](2026-10-09-metal-4-backend.zh.md)所描述的平台选择部署，
在 RHI 内通过 target-specific dependencies 与 `cfg` 为 macOS 选择 Metal，
为 Windows/Linux 选择 Vulkan。调用方依赖 `moonfield-rhi`，不固定后端。
将依赖可用性与实现选择分开：Cargo feature 具有累加性，多个调用方不应被要求协调互斥的
后端 feature。运行时选择和同平台后端覆盖不属于本提案的要求。

只有契约确实一致时，才将后端无关算法移入 `shared/`。分配偏移、帧上传策略和资源延迟回收
记账是候选项；原生分配、复制和完成查询由后端负责。保留
[RHI 的所有权和公共类型规则](../../../../crates/moonfield-rhi/AGENTS.md)。
共享策略不能削弱资源生命周期或同步保证。

[core crate](../../../../crates/moonfield-rhi/core/Cargo.toml) 还链接了 Slang。
如果运行时只加载预编译 shader，将词汇与编译能力分开才有实际收益。此时应让编译能力可选；
只有离线工具需要复用它时，才保留或提取独立 compiler crate。
仅为了减少 crate 数量，不应让所有运行时构建都引入这一依赖。

迁移顺序：

1. 保留后端子 crate，在公共 RHI 接口上定义并测试内存、命令、同步及 shader 目标契约。
2. 将 Vulkan 与 Metal 适配到这些契约，集中后端选择，并在各自平台上使用相同调用方进行编译。
3. 统一接口建立后，重新评估包隔离。如果独立后端包没有实际调用方，将其移为私有模块，
   并更新公共类型检查器、workspace 成员、CI 和对应文档。

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

## Acceptance criteria

- 两套实现下的共享公共类型具有一致的含义、签名、所有权约束和可观察行为。
  可选能力有显式支持检查和明确的失败行为。
- render 和 ml 调用方可以针对选中的平台后端编译，不包含后端特有导入、硬编码的 Vulkan
  依赖 feature 或运行时 shader 目标。editor 剩余的平台特有集成由 Metal 提案跟踪，
  不能因本次重构就视为已经完成。
- 同一套契约测试通过公共 RHI 接口覆盖分配与回读、录制与复用、提交完成、同步和
  root data 快照。在各后端符合要求的硬件上运行；因硬件条件跳过不能证明行为等价。
- 保留验证原生实现不变量的后端内部测试。格式检查、Clippy、Agent Note 检查、
  公共类型检查器及平台构建检查在改动后的布局下通过。
- 如果后端改为私有模块，所选目标的依赖图不包含未选中的原生绑定。workspace 与 CI 命令
  反映新的包布局，公共接口不新增后端类型。

## Risks

私有模块会失去独立后端编译单元，以及 crate 级依赖和可见性限制。target 条件、私有模块
可见性及检查需要承担这些职责；如果取舍不值得，则保留子 crate。
包列表更短不代表构建更快。

适配偏向 Vulkan 的契约，可能迫使 Metal 进行代价高昂或不正确的模拟。
根据 render 和 ml 的实际用法定义必要行为，再验证两套实现。内存类别和同步改动尤其需要
谨慎，因为方法名称一致不能证明 GPU 可见性或完成语义一致。

RHI 自有公共层可能退化为大量浅层转发包装。每项适配必须统一行为、保证不变量，
或隐藏后端特有要求。只有证明生命周期与同步兼容后才共享算法，不能仅凭源文件相似就合并。

仅按平台选择会限制同平台测试和后端覆盖。新增 D3D12 或运行时回退时，
需要重新评估选择方式，同时保持公共资源和命令契约稳定。

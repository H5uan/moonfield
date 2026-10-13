# Agent Note: RHI backend private modules

Status: implemented

**Superseded (platform selection):** 双臂平台选择已被单一 Vulkan 后端取
代——见
[the Vulkan pivot note](2026-10-13-rhi-vulkan-pivot.zh.md)。私有模块布局本身
继续成立。

[English](2026-10-11-rhi-backend-private-modules.md)

## Problem

[子 crate 决策](2026-10-09-rhi-backend-subcrates.zh.md)把后端隔离为独立包
（`moonfield-rhi-core`/`-vulkan`/`-metal`），藏在 facade 的 re-export 列表之后
—— 即[分层提案](../../implemented/architecture/2026-10-10-rhi-interface-and-backend-layering.zh.md)
第 1 步规定的迁移起点。契约建立之后（[同步/内存](2026-10-11-rhi-sync-memory-contract-narrowing.zh.md)
与[图形录制](2026-10-11-rhi-graphics-recording-contract.zh.md)契约、平台选择的
消费方），提案第 3 步要求重估：若独立后端包没有具体消费者，就转为私有模块。

审计结论：facade 之外没有任何 crate 依赖子 crate，CI 全部走 workspace 级命令
（无按 crate 的引用），子 crate 的独立编译单元不服务任何消费者。唯一需要私有
模块保留的属性 —— crate 内 GPU 测试 —— 随后端一起作为 `#[cfg(test)]` 模块迁移。

## Decision

`moonfield-rhi` 是一个拥有公共接口的 crate：

- 原 `moonfield-rhi-core` 溶入 facade 根：`src/types.rs`、`src/error.rs`、
  `src/indirect.rs`、`src/shader/`（词表与共享 Slang 编译器）。
- 后端是私有模块：`src/backend/vulkan/`（ash）与 `src/backend/metal/`
  （Metal 4），由平台在编译期选择 —— `backend/mod.rs` 中的
  `#[cfg(not(target_vendor = "apple"))]` / `#[cfg(target_vendor = "apple")]`。
  facade 的 re-export 列表承载各平台的公共项。
- 按[分层提案]，后端选择在 RHI 内部按平台进行：消费方依赖 `moonfield-rhi`
  而不指定后端。`vulkan`/`metal` cargo feature 取消（连同互斥 guard 一起）；
  `validation` feature 保留为开发便利（Apple 目标上是 no-op）。
- 原生栈在 `Cargo.toml` 中按目标门控（非 Apple：ash、ash-window、
  gpu-allocator；Apple：objc2 栈），被选平台的依赖图因此不含未选后端的绑定
  —— 子 crate 承载的 crate 级依赖限制变为平台级。
- crate 内 GPU 测试保留 `pub(crate)` 访问：它们是私有后端的 `#[cfg(test)]`
  模块（冒烟测试拼写为 `cargo test -p moonfield-rhi
  backend::vulkan::gpu_tests::headless_triangle`）。

取代上方子 crate 布局决策；边界规则不变（facade 的 re-export 列表即全部公共
表面，由 `scripts/verify_rhi_boundary.py` 机械强制，现在扫描单一 `src/` 树）。

## Alternatives considered

- **保留子 crate。** 弃用原因：不存在独立消费者，跨 crate 接口没有换来任何
  东西，只多了一份 members 列表；crate 内 GPU 测试作为私有模块存活。
- **保留 `vulkan`/`metal` feature 供开发侧交叉选择。** 弃用原因：平台选择即
  部署模型（分层提案的要求）；一个能选出另一平台后端的 feature 只会引入永不
  发布的配置。搁置的 Metal bindless 工作恰好说明这一点 —— 无论哪种方式
  editor 在 macOS 上都不构建。

## Consequences

- 任何后端改动都重编译 RHI crate —— 放弃了子 crate 的独立编译单元属性
  （分层提案的风险记录早有预料：包列表变小不等于构建变快）。
- 后端分离由 `cfg` 门控取代 crate 级可见性；`verify_rhi_boundary.py` 与
  私有模块可见性承担子 crate `pub` 边界曾经结构性强制的那份纪律。
- workspace members 列表只剩 `crates/*` glob；CI 无需改命令（本来就是
  workspace 级）。

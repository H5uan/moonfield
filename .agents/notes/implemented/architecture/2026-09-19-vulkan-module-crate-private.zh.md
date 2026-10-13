# Agent Note: The vulkan module tree is crate-private

Status: implemented

[English](2026-09-19-vulkan-module-crate-private.md)

## Problem

`moonfield-rhi` 的 `lib.rs` 声明的是 `pub mod vulkan`，整个后端模块树因此公开可达
（`moonfield_rhi::vulkan::device::…`），尽管 [RHI 边界](2026-08-19-vulkan-rhi-boundary.md)
的本意是 crate 根为唯一对外面。边界靠的是树内每个成员各自写成 `pub(crate)`：任何
新增到树里的 `pub` 项——以及 `vulkan/mod.rs` 里漏掉的任何再导出——都会为同一批
类型泄漏出第二条未经筛选的路径，`scripts/verify_rhi_boundary.py` 是唯一的兜底。

## Decision

声明改为 `pub(crate) mod vulkan`；crate 根保留 `pub use vulkan::*`，它恰好再导出
`vulkan/mod.rs` 里那份筛选过的清单。对外面——即下游 crate 导入的那些名字——完全
不变，因此 crate 之外没有任何调用点改动。crate 内部（包括 crate 内的 `gpu_tests`）
同样无变化：`crate::vulkan::…` 路径的解析与之前相同。

让这次改动成本极低的导入审计结论：下游每一处引用都走 `moonfield_rhi::<Name>` 或
`moonfield_rhi::types::…`；整个工作区没有任何 crate 引用 `moonfield_rhi::vulkan`。

## Alternatives considered

- **保留 `pub mod vulkan`，靠评审把关成员可见性。** 否决：意外公开泄漏正是这条
  边界要防的失败模式，而纯评审的执行已经产生了本笔记所关闭的漂移。
- **把筛选后的再导出从 `vulkan/mod.rs` 挪到 `lib.rs`。** 否决：这份清单记录的是
  后端对自身对外面的看法，放在它所挑选的模块旁边；挪到根部等于为同一个事实造
  第二个家。
- **拍平模块树（取消 `vulkan` 模块）。** 否决：`src/vulkan/` 内部的模块划分
  （device、swapchain、sync……）是这个 crate 的工作组织方式；为一次可见性改动而
  重命名所有文件是纯粹的 churn。

## Consequences

- `moonfield_rhi::vulkan::…` 在 crate 外不再可解析；`vulkan/mod.rs` 里的根部再导
  出清单成为公开后端面的单一定义，`verify_rhi_boundary.py` 继续约束这些名字暴露
  的内容。
- 树内 `pub` 但不在筛选清单中的类型现在在下游不可达；要提升某一个就是显式加一
  行再导出。
- 提到 `vulkan` 模块路径的文档引用与 SAFETY 注释描述的是 crate 内部布局，依旧准
  确。

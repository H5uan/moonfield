# Agent Note: Log-crate layering boundary — leaf crates use tracing directly

Status: implemented

[English](2026-09-05-log-crate-layering-boundary.md)

## Problem

`moonfield-log` 依赖 `moonfield-app`（仅为 `LogPlugin` 的 `App`/`Plugin` 实现），
而最底层的 Vulkan crate `moonfield-rhi` 依赖 `moonfield-log`。传递闭包把整个框架层
（`app` → `ecs` → `time` → `base`）拖进了 RHI，导致 RHI 无法脱离它对之一无所知的
ECS 框架独立构建、测试或复用。

## Decision

采用参考实现自身的结构（`bevy_log` 依赖 `bevy_app`；`bevy_ecs` 等底层 crate 直接
使用 `tracing`）：

- `moonfield-rhi` 直接依赖 `tracing`；其 11 处 `moonfield_log::{error, warn, info}!`
  调用点机械地改写为 `tracing::…`。行为不变：这些宏本来就是 `tracing` 的转口
  重导出，输出格式由 `LogPlugin` 安装的进程级全局 subscriber 决定，与发送方
  crate 无关。
- `moonfield-log → moonfield-app` 保留：`LogPlugin` 是框架层设施，它唯一的消费者
  （`moonfield-editor` 的 `main.rs`）本就依赖 `moonfield-app`。
- 边界规则记录在 `crates/AGENTS.md`：必须留在框架之下的 crate（`moonfield-rhi`、
  `moonfield-math` 及未来的叶子 crate）直接使用 `tracing`，
  永远不依赖 `moonfield-log`。`*_once!` 宏在 `moonfield-log` 中；若某个叶子 crate
  真的需要它们，那是重新审视分层的信号，而不是添加依赖的理由。

## Alternatives considered

- **把 `LogPlugin` 移入 `moonfield-app`，让 `moonfield-log` 成为零依赖叶子。**
  否决：偏离参考实现的结构（`bevy_log` 是框架 crate），需要把约 130 行代码及
  tracing-subscriber/tracing-log/tracing-error 依赖和 `trace` feature 搬进
  `moonfield-app`，而相比切断那一条有害的边并无额外收益。
- **在 `moonfield-log` 中用 feature 门控 `moonfield-app` 依赖。** 否决：Cargo 的
  feature 统一机制意味着 editor 构建会为整个依赖图启用该 feature，于是每次工作区
  构建中 `moonfield-rhi` 仍会传递依赖 `moonfield-app`；解耦只在独立构建时成立。

## Consequences

- `cargo tree -p moonfield-rhi` 不再包含 `moonfield-app`、`moonfield-ecs`、
  `moonfield-time`、`moonfield-log`；RHI 的依赖锥只剩 `moonfield-math`（及其
  `moonfield-reflect`）和外部 crate。
- 日志输出格式、级别过滤（`RUST_LOG=moonfield_rhi=…`）、模块 target 完全不变——
  宏就是同一批 `tracing` 宏。
- `render-core` / `render-feature` / `winit` / `editor` 继续使用 `moonfield-log`
  （它们本就依赖 `moonfield-app`，这条边在那里无害）。

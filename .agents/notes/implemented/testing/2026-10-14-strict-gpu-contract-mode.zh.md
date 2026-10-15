# Agent Note: Strict GPU contract mode

Status: implemented

[English](2026-10-14-strict-gpu-contract-mode.md)

## Problem

托管 CI 缺少受支持硬件是正常情况，但未执行契约便返回的 GPU 测试仍会计为通过。
只在 Vulkan 后端内部使用的严格辅助函数无法阻止公开 RHI 契约、渲染 feature、
编辑器渲染和 ML 测试在合格 runner 上跳过。

## Decision

**所有套件共用一个公开测试策略。** `moonfield_rhi::testing::skip_gpu`
在宽容模式下报告跳过，在设置 `MOONFIELD_STRICT_GPU` 时携带原因 panic。
后端内部的设备和能力探测调用它；公开 RHI 契约、render-core、mesh/splat、编辑器
及 ML 测试也调用它。辅助函数不包含后端类型，自身不依赖 GPU。

**严格任务运行所有 feature 路径。** 手动 `gpu-contract` 工作流指向
`self-hosted, gpu`，在严格模式下执行
`cargo test --workspace --all-features --locked`。托管 CI 保留宽容行为。
环境开关通过子进程测试，测试不修改进程级环境状态。

## Alternatives considered

**让托管 CI 变严格。** 未采用：托管硬件不满足引擎的 GPU 要求。
严格契约需要合格 runner。

**每个 crate 复制跳过辅助函数。** 未采用：独立辅助函数可能对严格模式的处理
不一致，使新增套件不受保护。

**单独的严格测试二进制或解析跳过输出。** 未采用：维护平行套件会与被测试接口
漂移，解析输出也不如在跳过位置直接失败。

## Consequences

- 合格 runner 满足套件涉及的 Vulkan descriptor heap、device-address commands、
  float32 atomics 和 uploader 要求。
- 严格模式下，初始化或能力探测失败不能让任何 GPU 套件静默通过。
- 公开策略及其环境行为无需 GPU 硬件即可测试；普通单元测试不需要严格 runner。

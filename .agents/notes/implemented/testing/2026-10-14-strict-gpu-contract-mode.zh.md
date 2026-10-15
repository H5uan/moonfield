# Agent Note: Strict GPU contract mode

Status: implemented

[English](2026-10-14-strict-gpu-contract-mode.md)

## Problem

[架构契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
的测试项：rhi 的 GPU 集成测试在无 Vulkan 实例或设备、缺少必需的
`VK_EXT_descriptor_heap`、或缺少特性门控能力（`VK_KHR_device_address_commands`、
`shaderBufferFloat32AtomicAdd`、帧上传器）时以 eprintln 跳过——CI 因此把
跳过计为通过，本应运行契约测试的硬件上的回退不可见。

## Decision

**`MOONFIELD_STRICT_GPU` 把跳过变成失败。** `gpu_tests::common::skip`
在宽容默认下播报跳过；设置了该环境变量时改为携带跳过原因 panic，使测试
与任务失败。GPU 测试中的每个跳过点都经由它：实例与设备创建失败、
descriptor-heap 探测、特性门控能力、帧上传器创建。

**`gpu-contract` 工作流即严格任务。** 它是手动的（`workflow_dispatch`）
并指向合格 runner（`self-hosted, gpu` 标签）；GitHub 托管 runner 不合格，
会正确地失败。它带着 `MOONFIELD_STRICT_GPU` 运行
`cargo test --workspace --locked`，覆盖公开契约 GPU 测试与代表性的 mesh、
splat 路径（`offscreen_triangle`、`gaussian_fit`）。常规 CI 保持宽容默认。

## Alternatives considered

- **让常规 `test` 任务变严格。** 舍弃：`VK_EXT_descriptor_heap` 只由较新的
  NVIDIA 驱动实现，托管 runner 会在每次推送上失败。严格契约需要自己的
  合格硬件。
- **单独的严格测试二进制。** 舍弃：同一批测试已经编码了契约，平行套件会
  与之漂移。环境变量在现有跳过点切换行为。
- **在工作流里统计跳过数并对非零总数失败。** 舍弃：解析测试输出脆弱；
  panic 在跳过点携带原因失败，落在测试自身的报告里。

## Consequences

- 合格 runner 由测试的要求定义：具备 `VK_EXT_descriptor_heap`、
  device-address commands、buffer float32 atomics 和可用帧上传器的 Vulkan
  设备。
- `skip_forced` 与环境变量读取分离，严格行为无需改动进程级环境状态即可
  单元测试。

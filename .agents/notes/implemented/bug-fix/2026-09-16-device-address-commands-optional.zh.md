# Agent Note: Device-address commands become optional

Status: implemented

[English](2026-09-16-device-address-commands-optional.md)

## Problem

[commit 4f1c3ec](../feature/2026-09-12-device-address-commands-and-gpu-timestamps.zh.md)
把 `VK_KHR_device_address_commands` 列为无条件必需的扩展，并断言所有目标
驱动（包括开发机的 T1000）都提供它。这个论断从未对照真实硬件验证过：
开发机（Quadro T1000 Mobile，驱动 610.43.02，NVIDIA ICD 1.4.341）上运行
`vulkaninfo`，三个物理设备（T1000、Intel RPL-S、llvmpipe）没有一个暴露该
扩展（`VK_NV_device_address_commands` 同样缺失）。该扩展是 2024 年从
`VK_NV_device_address_commands` 提升为 KHR 的，NVIDIA 只在 RTX 20 系及
更新的显卡上提供；入门级 Turing（T1000）在其门槛之外。结果就是触发本
note 的启动失败：`RenderPlugin could not initialize Vulkan: device request
failed: physical device is missing required extensions:
["VK_KHR_device_address_commands"]`，恰好发生在编辑器日常开发的机器上。

## Decision

- `VK_KHR_device_address_commands` 从 `REQUIRED_DEVICE_EXTENSIONS` 移入
  `OPTIONAL_DEVICE_EXTENSIONS`，整组作为能力位（基于设备地址的间接
  draw/dispatch、`cmd_memcpy`、GPU 地址时间戳解析）。不新增句柄形态的回退。
- 门控与既有 RT / 原子浮点模式一致，三处同时生效：扩展仅在枚举到时进入
  设备启用列表；其 feature 结构仅在启用时接入 pNext 链；
  `DeviceExtensionFunctions::device_address_commands` 改为 `Option`，仅在
  扩展启用时才构建加载器。
- `CommandBuffer` 的五个地址命令（`draw_indirect`、`draw_indirect_count`、
  `dispatch_indirect`、`cmd_memcpy`、`resolve_timestamps`）保持纯 `GpuPtr`
  签名，加载器为 `None` 时以清晰消息 panic。`TimestampQueryPool::new` 在缺
  扩展的设备上返回 `Error::Unsupported`，而不是造一个结果永远读不出来的
  pool。
- `Device::device_address_commands()` 是能力查询（与
  `buffer_float32_atomic_add` 同形）；四个录制这些命令的 GPU 测试
  （`bindless_memcpy_dispatch_indirect`、`indirect_draw`、`timestamps`、
  `upload_ring`）在其为 false 时带明确原因跳过。
- `Stage::DRAW_INDIRECT` / `Access::INDIRECT_COMMAND_READ` 屏障词汇保留：
  它是不依赖扩展的普通 stage/access 对。

## Alternatives considered

- **句柄形态的回退命令。** 把 `GpuPtr` 反查回 buffer 句柄需要一张地址反查
  注册表，或双形态命令 API——两者都违背地址优先的单一机制不变量，且目前
  没有任何生产调用方需要它。
- **保持扩展为必需。** 失败依旧：开发机上的任何设备都无法创建逻辑设备，
  编辑器与全部 GPU 测试都被挡住。
- **升级驱动或更换硬件。** 不是修复：已装驱动远超任何支持门槛，入门级
  Turing 硬件上该扩展确实不存在。
- **回滚 2026-09-12 的提交。** 恢复句柄+偏移签名与主机同步的时间戳路径，
  丢弃尚无生产消费方的 GPU-driven 地基——为不支持它的硬件付出了全面回退
  的代价。

## Consequences

- 编辑器与渲染管线在开发机上恢复初始化；四个录制地址命令的 GPU 测试在
  该机器上跳过并报告驱动的实际能力。
- 地址命令的调用方必须先查 `Device::device_address_commands()`；误用会大声
  失败（录制时 panic、建 pool 时 `Unsupported`），而不是带着空函数指针提交。
- 2026-09-12 note 中"所有目标驱动（包括 T1000）都提供它"的结论被取代：
  入门级 Turing 并不提供。
- 在确实暴露该扩展的驱动（RTX 20 系及更新）上，GPU-driven / GPU 地址剖析
  路径不变——optional 只是放宽了设备下限，不回归有能力硬件的表现。
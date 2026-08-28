# Agent Note: 可选设备扩展与点名式缺失报错

Status: implemented

[English](2026-08-28-optional-device-extensions.md)

## Problem

`DEVICE_EXTENSIONS` 无条件要求整套 ray-tracing 扩展。任一缺失都会让 `vkCreateDevice` 以笼统的 `ERROR_EXTENSION_NOT_PRESENT` 失败——既不知道缺哪个，也无法以功能降级的方式运行。这是真实配置而非边角情形：Turing 级 NVIDIA 卡（如 T1000）完全不提供 KHR RT 扩展，而软件渲染器（llvmpipe）反而有；mesh 渲染和编辑器核心 pass 从不触碰 RT。

## Decision

`moonfield-rhi` 的设备创建把扩展拆成两张表：

- `REQUIRED_DEVICE_EXTENSIONS` —— 受支持设备必须暴露的 8 个扩展（swapchain、descriptor heap、extended dynamic state3、mesh shader、mutable descriptor type、dynamic vertex input、device generated commands）。缺失即 `Error::DeviceRequest` 失败，并点名列出每个缺席扩展，取代难以诊断的 `ERROR_EXTENSION_NOT_PRESENT`。
- `OPTIONAL_DEVICE_EXTENSIONS` —— 以组为单位的 RT stack（`acceleration structure`、`ray tracing pipeline`、`ray query`、`position fetch`，及其共享前置 `pipeline library` + `deferred host operations`）外加 `invocation reorder`。每个扩展仅当物理设备暴露时才启用，否则以 `warn!` 跳过。

对应的 `PhysicalDevice*Features` 结构仅在扩展被启用时挂到 `features2` 链上——feature 请求始终与启用列表一致。`Device::optional_extension_enabled` 让消费方查询能力并按需降级，而不是失败。

## Alternatives considered

**全部保持必需。** 否决：让无 RT 的卡也能跑图形是本版本实际验证的环境要求。

**首选设备缺扩展时回退到其它物理设备（llvmpipe）。** 否决：始终选独显；软件回退会掩盖缺失扩展的诊断，且渲染效果差。

## Consequences

- T1000 级卡以 RT 禁用状态启动编辑器（设备创建时六条警告）；mesh、splat 与 UI pass 不受影响。
- 缺失的必需扩展以名字形式呈现在错误与日志中。
- RT 功能代码可先查 `optional_extension_enabled` 再创建管线；当前消费方还没有基于它的门槛。
- `submit_frame_timeline`（timeline 帧循环的设备侧提交，见 `feat(render): timeline semaphore frame loop`）位于同一文件；其契约见对应 note。
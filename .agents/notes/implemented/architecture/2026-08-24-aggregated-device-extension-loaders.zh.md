# Agent Note: Aggregate device extension loaders

Status: implemented

[English](2026-08-24-aggregated-device-extension-loaders.md)

## Problem

逐 draw 的动态状态需要 Vulkan 从未晋升到核心的命令:
`vkCmdSetColorBlendEnableEXT` / `ColorBlendEquationEXT` / `ColorWriteMaskEXT`
属于 `VK_EXT_extended_dynamic_state3`,因此 ash 把它们暴露在独立的
`ext::extended_dynamic_state3::Device` loader 上,而不是 `ash::Device` 上
(它只覆盖核心 + 已晋升命令)。RHI 最初把该 loader 存成 `Device` 上的临时
单字段,通过逐扩展的 getter 克隆进每个 `CommandPool`/`CommandBuffer`,并包
进 wgpu 风格的 `ExtensionFn<T>` 枚举(`Extension`/`Promoted`);但本 RHI
用到的扩展都没有核心对应物,`Promoted` 成了死代码。扩展一多,"单字段 +
getter"的模式就会变成一堆平铺,也没有一个集中位置记录所有已加载的扩展。

## Decision

沿用 wgpu-hal `vulkan/mod.rs` 的形态,并按本 RHI 的实际需求简化:

- `DeviceExtensionFunctions` —— 聚合所有 loader 的结构体,以
  `Arc<DeviceExtensionFunctions>` 存在 `Device` 上,在
  `Device::from_physical_device` 中只构建一次(与 wgpu 的
  `DeviceExtensionFunctions` 在 `Arc<DeviceShared>` 内的形态一致)。
- `CommandPool`/`CommandBuffer` 持有这个 `Arc` —— 在 pool 创建时克隆一次,
  被每个 command buffer 共享。loader 是函数指针表;克隆 `Arc` 不复制任何表。
- 调用点直接字段访问 loader:
  `self.ext.extended_dynamic_state3.cmd_set_*` —— 字段访问,不再有逐扩展
  getter。`CommandBuffer` 从不复制表,通过共享的 `Arc` 解引用。

不使用 `ExtensionFn<T>`:这个带 `Promoted` 分支的枚举没有任何构造点。将来
某个扩展真正被晋升(目前没有)时,再把该 loader 字段换成不带
`#[allow(dead_code)]` 的标记 —— 在此之前 YAGNI。

## Alternatives considered

- **`ExtensionFn<T>`(`Extension`/`Promoted`)+ 逐扩展 getter。** 先试行后
  在评审中否决:`Promoted` 从未被构造(`-D warnings` 下是死代码);并且每个
  调用点要先经 `self.ext_dynamic_state3()` 跳一步才能到真正的命令 —— 对比
  wgpu 从不写 getter,显得啰嗦。
- **保留单点临时 loader 字段。** 每扩展的代价:加字段、加 accessor,再从
  `Device` → `CommandPool` → `CommandBuffer` 层层传递;多扩展时同样变成
  平铺;否决。
- **只把整张表存在 `CommandBuffer` 上。** loader 是设备作用域的概念;从设备
  以 `Arc` 共享保持单一真源,也让其他设备消费者(如未来的 GPU-driven 录制
  器)能拿到同一张表。

## Consequences

- 新增扩展 loader 现在只需:在 `DeviceExtensionFunctions` 加一个字段、在
  `from_physical_device` 加载一次、在 pool 创建时克隆一次 `Arc` —— 完成。
- `CommandPool`/`CommandBuffer` 以 `Arc` 共享表;热路径 draw 只通过共享
  指针解引用一次,与本地字段的间接调用成本相同。
- `CullModeFlags` 与 depth 状态仍留在 `ash::Device` 核心方法上 —— 它们是
  Vulkan 1.3 核心、真正被晋升的,与扩展表分开放是刻意的。
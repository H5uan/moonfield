# Agent Note: Address-based commands and GPU-address timestamps (VK_KHR_device_address_commands)

Status: implemented

[English](2026-09-12-device-address-commands-and-gpu-timestamps.md)

## Problem

命令接口原来对 indirect draw/dispatch 和缓冲拷贝一律接收 buffer 句柄加
偏移（`cmd_draw_indirect(buffer, offset, …)`），而引擎的内存模型是
地址优先——`GpuAllocation` 的 buffer 只是地址载体，句柄加偏移不过是
把调用方已有的 `GpuPtr` 复述一遍。crate 也没有 GPU 性能剖析路径：经典
时间戳查询的终点是一次 `vkGetQueryPoolResults` 主机同步。

## Decision

- `VK_KHR_device_address_commands` 列为必需设备扩展（缺失时设备创建
  失败并列出名称，与所有必需扩展一致）；请求其唯一的 feature 位，
  加载器并入 `DeviceExtensionFunctions`。
- indirect/copy 命令改为消费设备地址：
  `draw_indirect(args: GpuPtr, draw_count, stride)`（offset 参数移除——
  调用方用 `GpuPtr::offset`）、
  `draw_indirect_count(args: GpuPtr, count: GpuPtr, max_draw_count, stride)`、
  `dispatch_indirect(args: GpuPtr)`，以及经 `vkCmdCopyMemoryKHR` 的
  `cmd_memcpy(dst: GpuPtr, src: GpuPtr, size)`。所有地址范围都带
  `FULLY_BOUND` 标志——`GpuAllocation` 的 buffer 是完整绑定的。
- barrier 词汇表新增 `Stage::DRAW_INDIRECT`，与既有的
  `Access::INDIRECT_COMMAND_READ` 配对，表达 indirect 参数的 hazard。
- `sync.rs` 新增 `TimestampQueryPool`：`new(device, count)`，以及
  `timestamp_period_ns`（物理设备 `limits.timestampPeriod`，创建设备时
  缓存在 `Device` 上）。`CommandBuffer` 新增 `reset_timestamps`（在
  一段 pass 写时间戳之前做的核心 pool 重置）、
  `write_timestamp(queries, index, stage)`（sync2 `vkCmdWriteTimestamp2`，
  约定传单一 stage）、`resolve_timestamps(queries, first, count,
  dst: GpuPtr)`——`vkCmdCopyQueryPoolResultsToMemoryKHR` 写出连续的
  `u64` tick 值（64 位结果带 `WAIT`）。CPU 在提交的 timeline 点之后
  读映射内存；全程无主机同步。

## Alternatives considered

- **手动 `get_device_proc_addr` 加载。** 只有在 ash 没有该扩展时才
  需要；锁定的 ash master 自带加载器，聚合的 `DeviceExtensionFunctions`
  模式即可覆盖。
- **可用性成对结果（`WITH_AVAILABILITY`）代替 `WAIT`。** 纯数值加
  `WAIT` 与参考实现同形，且结果体积减半；resolve 记录在同一提交内、
  位于写入之后，可用性已有序。
- **保留句柄/偏移签名、内部再解析句柄。** 同一内存模型出现两套命令
  形态；地址形式还省掉了 offset 参数，与 crate 其余部分的指针模型
  一致。

## Consequences

- indirect/copy/timestamp 路径的公开命令不再出现 buffer 句柄；
  `GpuAllocation::buffer()` 收缩为 upload/readback 的内部细节。
- GPU 时间戳不需要主机同步：剖析数据直接落进映射内存，用
  `timestamp_period_ns` 换算。
- 不支持该扩展的设备在创建设备时失败并列出名称；所有目标驱动
  （包括 T1000）都提供它。
- `gpu_tests::timestamps` 端到端覆盖 写入 → resolve → 主机读取；
  `indirect_draw` 与 `bindless_memcpy_dispatch_indirect` 覆盖地址形式的
  命令。

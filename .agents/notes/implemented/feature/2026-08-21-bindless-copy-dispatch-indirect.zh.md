# Agent Note: Bindless copy and indirect dispatch commands

Status: implemented

[English](2026-08-21-bindless-copy-dispatch-indirect.md)

## Problem

bindless 里程碑（仅计算）已交付 `GpuAllocation`、计算管线和根指针
dispatch，但原始 scope 里的两条命令尚未接通：GPU→GPU 内存拷贝
（`cmd_memcpy`）和从 GPU 内存读取 dispatch 参数（`dispatch_indirect`）。
博客中的 `gpuMemCpy` 与 `gpuDispatchIndirect` 接受裸 GPU 地址——Vulkan
无法直接满足这种形态：`vkCmdCopyBuffer2` 与 `vkCmdDispatchIndirect` 都
要求 buffer 对象加 offset，从不接受裸地址。

## Decision

这两条命令以 `&GpuAllocation` 为参数，而非裸 `GpuPtr`。该 allocation
本来就是这个内存优先模型的地址载体——它拥有底层 `vk::Buffer`、CPU 视图
和设备地址，因此对于 Vulkan 强制以 buffer 对象为基础的命令，它是自然的
入参。

- `GpuAllocation::buffer()` 暴露底层 `vk::Buffer`，供命令层提交给
  `vkCmdCopyBuffer2` / `vkCmdDispatchIndirect`。
- `GpuAllocation::new` 现在创建载体 buffer 时在 `SHADER_DEVICE_ADDRESS`
  之上附加 `TRANSFER_SRC | TRANSFER_DST`。传输标志是功能必需——校验层会
  拒绝源缺少 `TRANSFER_SRC` 或目标缺少 `TRANSFER_DST` 的拷贝——而且它是
  buffer 创建时的静态能力，不是逐资源的运行时状态追踪，因此 bindless 模型
  （无资源列表、无状态追踪）不受影响。
- `CommandBuffer::dispatch_indirect(&GpuAllocation)` 记录
  `vkCmdDispatchIndirect`，从 allocation 基址读取 `DispatchIndirectArgs`
  （x/y/z）。注意：由先前 dispatch 写入的参数需要 compute→compute barrier
  才能被命令处理器看到；CPU 写入的参数在 `queue_submit` 之后即可见，
  无需显式 barrier。`HAZARD_DRAW_ARGUMENTS`（indirect multi-draw 里程碑）
  依然不在范围内。
- `cmd_memcpy(dst, src, size)` 记录 `vkCmdCopyBuffer2`（sync2），拷贝整块
  allocation，两侧 offset 均为 0。仅整块拷贝；子区域拷贝留待未来需要。
  transfer→consumer barrier 由调用方负责。

测试（`tests/bindless_memcpy_dispatch_indirect.rs`）在 lavapipe 上把两条
命令验证为完整的 CPU→GPU→CPU 往返：memcpy 让目标填满相同值，indirect
dispatch 用读自 GPU 内存的工作组数启动 `+1` 内核。

## Alternatives considered

- **用 compute 拷贝内核实现 `cmd_memcpy`，保持 API 纯地址。** 否决：着色器
  拷贝更慢（走着色器路径而非传输引擎），需要额外的内核/管线，并与已有的
  `Stage::TRANSFER` 语义冲突。博客本身就把持久/大块拷贝交给驱动的拷贝
  命令，而它在 Vulkan 中即基于 buffer 对象。
- **从 `GpuAllocation` 暴露 `(buffer, offset)` 二元组，重建 handle+offset
  API。** 否决：那正是 bindless 模型要移除的 retained-mode 形态。
  `&GpuAllocation` 让 API 保持在双指针（`HostPtr`/`GpuPtr`）上，buffer
  对象仍是内部载体。
- **现在就加子区域拷贝与 offset 参数。** 暂缓：还没有消费者需要它，整块
  拷贝即最小可验证单元。

## Consequences

- 每个 `GpuAllocation` 的 buffer 都获得传输能力，包括 `Memory::Gpu`
  （设备本地）输出，为博客的 upload→private-heap 拷贝模式铺路。
- `dispatch_indirect` 使 dispatch 参数可以间接化（CPU 或 GPU 写入）——
  这是 GPU-driven 计算所需的一半；GPU 生成绘制参数（带 hazard flag 的
  barrier）仍属于 indirect multi-draw 里程碑。
- 命令层现在依赖 `GpuAllocation::buffer()`；buffer 的存活期与 allocation
  本身一致，不新增生命周期面。
- clippy/fmt 干净，测试在 lavapipe（Linux CI）通过；MoltenVK/lavapipe 的
  兼容性不变（两者都支持 sync2 拷贝与 indirect dispatch）。
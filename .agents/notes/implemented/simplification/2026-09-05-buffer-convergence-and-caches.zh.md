# Agent Note: One buffer vocabulary, cached shaders, cached pipelines

Status: implemented

[English](2026-09-05-buffer-convergence-and-caches.md)

## Problem

bindless 迁移后两种缓冲词汇并存：`Buffer`（usage 声明、固定功能时代）与
`GpuAllocation`（带 CPU/GPU 双视图的 BDA 载体）。egui 拉取化之后，
`Buffer` 唯一的生产用户是 offscreen 读回。同时每个管线构造器都自起一个
Slang 编译器、从头编译着色器（core-3D 管线每次构建三次编译），且管线
创建在每次进程启动时都冷命中驱动。

## Decision

- `GpuAllocation` 吸收 `Buffer`：读回走 `GpuAllocation::read_bytes`
  （`Memory::Readback` 分配的映射视图）、indirect draw 收
  `&GpuAllocation`（载体本就带 `INDIRECT_BUFFER`）、`FrameUploader`
  只经 `upload_alloc` 暂存。`Buffer`、`BufferUsage`、uploader 的
  `upload`/`upload_and_wait` 删除——一种缓冲类型，其 usage 集覆盖
  bindless 模型触及内存的全部方式。
- `ShaderCache` 成为编译路径：值 `Arc` 化（跨线程共享）、
  `compile_file_reflection` 把反射与 SPIR-V 一并记忆化、缓存挂在
  `Device` 上作为惰性单例（`device.shader_cache()`），与 uploader、heap
  同列。`ShaderCache` 依不变量 `Send + Sync`：所有编译器访问都在它的
  某把 mutex 之下，缓存值是纯数据。两个管线构造器都经它编译。
- 一个 Vulkan `PipelineCache` 走同一模式：惰性创建、从
  `<XDG_CACHE_HOME 或 ~/.cache>/moonfield/pipeline_cache.bin` 播种、
  传给每个 graphics/compute 管线创建调用、`Device::drop` 时回写。被拒
  （过期/驱动更新）的缓存数据只损失一次冷启动。

## Alternatives considered

- **为读回与 indirect 参数保留 `Buffer`。** "一段 GPU 内存"两种词汇正是
  收敛评估否决的双轨；被吸收的便利只是一个读方法和一个参数类型。
- **进程级全局着色器缓存。** 设备拥有自己的单例；全局缓存在设备
  teardown 之后存活毫无收益（SPIR-V 与设备无关，但管线构建本就经过
  设备）。

## Consequences

- 一种缓冲类型：每个分配都带 `SHADER_DEVICE_ADDRESS | TRANSFER_SRC |
  TRANSFER_DST | INDIRECT_BUFFER`（heap 背载再加
  `DESCRIPTOR_HEAP_EXT`）——每个 flag 都有真实消费者。
- 重复管线构建（测试、编辑器重载）每个设备只编译一次每个
  （着色器、入口）；磁盘缓存命中时驱动侧管线编译跨进程跳过。
- offscreen 读回、`upload_ring`、`indirect_draw` 测试在分配路径上
  原样运行。

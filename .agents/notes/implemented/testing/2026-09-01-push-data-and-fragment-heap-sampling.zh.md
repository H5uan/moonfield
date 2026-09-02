# Agent Note: push data GPU consumption and fragment-stage heap sampling

Status: implemented

[English](2026-09-01-push-data-and-fragment-heap-sampling.md)

## Problem

Core 3D 根指针集成之后，bindless 2.0 还剩两个验证缺口：

1. `vkCmdPushDataEXT` 只有录制级测试（`push_data_records_cleanly`），没有证据表明
   经 `push_data` 写入的字节真正到达着色器的 push-constant 块。
2. 堆采样（无描述符集布局的 `ResourceDescriptorHeap` / `SamplerDescriptorHeap`）
   只在 compute 验证过；fragment stage——材质纹理将来要走的路径——没有覆盖。

## Decision

新增两个 headless 测试，均在真实驱动上通过：

- `command_push_data.rs::push_data_feeds_root_pointers`：plus-one compute kernel
  的两个 BDA 根地址经一次 16 字节 `cmd_push_data` 写入（与 `set_bindless_root`
  经 `cmd_push_constants` 推入的布局一致）；读回断言 out = in + 1。这实证了
  push data 与经典 push-constant bank 互为别名：绑定的 `ComputePipeline` 是经典
  layout、16 字节 push range、无 DESCRIPTOR_HEAP 标志，照样消费 push data。
- `bindless_graphics_heap_sampling.rs::fragment_heap_sampling_roundtrip`：无
  描述符集布局的图形管线在 fragment shader 里经 `ResourceDescriptorHeap[0]` /
  `SamplerDescriptorHeap[0]`（以 `spvDescriptorHeapEXT` capability 编译）采样
  4x4 红堆纹理，再乘以经 `Ptr<float4>` 根指针（一条 8 字节 FRAGMENT
  push-constant range）读到的白色 tint，像素读回断言目标中心为红色。

## Alternatives considered

- 要求管线带 `VK_PIPELINE_CREATE_2_DESCRIPTOR_HEAP_BIT_EXT` 才能消费 push data：
  规范中 push data 与 push-constant 状态互为别名（last setter wins），测试证实
  经典 layout 管线即可消费——无需标志，无需改 RHI。
- 图形版 `set_bindless_root`：无必要，`CommandBuffer::push_constants` 配
  `ShaderStages::FRAGMENT` 已覆盖，Core 3D pass 就是这么做的。

## Consequences

- 两个缺口均以 buffer 级和像素级证据闭环；图形管线依赖的 bindless 2.0 机制
  现已全部端到端验证。
- push-data bank 在本机驱动上证实可喂着色器，后续把 per-draw 根数据从
  `cmd_push_constants` 迁到 `cmd_push_data` 不再需要 RHI 侧工作。

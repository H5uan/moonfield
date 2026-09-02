# Agent Note: RHI 模块布局与公开表面清理

Status: implemented

[English](2026-09-02-rhi-module-layout-and-surface.md)

## Problem

bindless 改造完成后,RHI 的模块地图和公开表面留下了不一致。`bindless.rs`
变成了大杂烩:内存原语(`Memory`/`GpuPtr`/`HostPtr`/`GpuAllocation`)、屏障
词汇(`Stage`/`BarrierHazard`)、队列枚举(`QueueType`)和 compute 管线
全挤在一个文件里,而文件名本身已经冗余——RHI 现在只有一种资源模型,
不存在"非 bindless"的东西。`view.rs`(包装 `ash` 的 `TextureView`)位于
crate 根目录,游离在 crate AGENTS.md 要求的 `src/vulkan/` 边界之外。若干
公开 API 仍在泄漏原始 `vk::` 类型:`bind_compute_pipeline(vk::Pipeline)`、
`draw_indirect*` 一族接收 `vk::Buffer`/`vk::DeviceSize`、
`GpuAllocation::buffer()`、`TextureSlotDesc` 携带公开的
`vk::ImageViewCreateInfo` 字段、`write_buffer_descriptors` 接收
`vk::DeviceAddressRangeEXT`,以及旧的 `pipeline_barrier`。同时
`Device::queue()`/`QueueType` 是死代码——调用方用的是
`graphics_queue()`/`compute_queue()`。还有一处泄漏越出了 RHI crate:
`Buffer::new` 接收 `gpu_allocator::MemoryLocation`,迫使
`moonfield-render-feature` 和 `moonfield-editor` 直接依赖
`gpu-allocator`。

## Decision

- `vulkan/bindless.rs` 更名为 `vulkan/memory.rs`,只保留内存/指针模型:
  `Memory`、`GpuPtr`、`HostPtr`、`GpuAllocation`——CPU 视图 + 设备地址
  共处一个对象。
- `Stage` 和 `BarrierHazard` 移入已拥有 `Fence`/`Semaphore` 的
  `vulkan/sync.rs`;`ComputePipeline` 移入 `vulkan/pipeline.rs`,与
  `GraphicsPipeline` 并列。`QueueType` 和 `Device::queue()` 被删除。
- `src/view.rs` 移入 `src/vulkan/view.rs`;`TextureView` 经由
  `vulkan/mod.rs` 和 crate 根的 glob 再导出。
- 除了文档化的互操作 `raw()` 访问器,公开 API 不再接收或返回原始 `vk::`
  类型:`bind_compute_pipeline` 接收 `&ComputePipeline`,`draw_indirect*`
  一族接收 `&Buffer` + `u64` 偏移,`GpuAllocation::buffer()` 和旧的
  `pipeline_barrier` 改为 `pub(crate)`,两个管线的 `raw()` 也收紧为
  `pub(crate)`。
- 堆写入入口收为 crate 内部:`TextureSlotDesc` 字段改为 `pub(crate)` 并
  提供 `pub(crate)` 构造函数,`write_resource_descriptors` 改为
  `pub(crate)`(其唯一的生产写入方是 `Texture::bindless` 和
  `OffscreenTarget`),`write_buffer_descriptors` 改用新的公开词汇
  `BufferRange { address: GpuPtr, size: u64 }`,取代
  `vk::DeviceAddressRangeEXT`。
- `vulkan/mod.rs` 显式再导出重要类型
  (`memory::{GpuAllocation, GpuPtr, HostPtr, Memory}`、
  `pipeline::{BlendMode, ComputePipeline, GraphicsPipeline, ShaderStageDesc}`、
  `sync::{BarrierHazard, Fence, Semaphore, Stage}`、`view::TextureView`、
  `descriptor_heap::BufferRange`),测试和下游 crate 从 crate 根导入。
  descriptor-heap 测试的图像写入用例转移到公开的
  `write_buffer_descriptors`;图像描述符覆盖由端到端采样测试继续承担。
  `docs/architecture.md` 中过时的 push-constant 句子改为描述
  `DrawData`/`FrameDrawArena`/`push_data` 的现状。
- `Buffer::new` 改收 crate 自己的 `Memory` 类别,取代
  `gpu_allocator::MemoryLocation`(内部经 `Memory::to_location` 映射);
  `Buffer::size()` 返回 `u64`,`Buffer::location()` 更名为
  `Buffer::memory()`。`moonfield-render-feature` 和 `moonfield-editor`
  不再直接依赖 `gpu-allocator`。

## Alternatives considered

- **保留 `bindless.rs` 为单一模块。** 否决:既然 RHI 只有一种资源模型,
  名字已经冗余,而且该文件混杂了四种关注点(内存原语、屏障词汇、队列
  枚举、compute 管线),它们各自都有自然的归属。
- **为测试保留公开的 `TextureSlotDesc`/`write_resource_descriptors`。**
  否决:唯一的生产写入方都在 crate 内(`Texture::bindless`、
  `OffscreenTarget`),测试覆盖转移到公开的 `write_buffer_descriptors`
  加上既有的端到端采样测试。
- **移除所有 `raw()` 访问器(`Device`/`Instance`/`CommandBuffer`/
  `Buffer`/`Semaphore`)。** 否决:它们是承重的互操作接缝,render-core
  的窗口循环和编辑器的 egui 后端都在用;只有管线的 `raw()` 收紧为
  `pub(crate)`。

## Consequences

- 模块地图与边界规则一致:所有 `ash` 类型都在 `src/vulkan/` 下,每个
  模块只命名一个关注点(memory、sync、pipeline、view)。
- 除了文档化的互操作 `raw()` 访问器,公开 API 不再泄漏 `vk::` 类型;
  `TextureSlotDesc` 不再被再导出。
- 测试套件的导入移到 crate 根
  (`moonfield_rhi::{ComputePipeline, GpuAllocation, Memory, Stage, ...}`),
  `indirect_draw` 通过 `&Buffer` 而非原始句柄录制。
- `docs/architecture.md` 不再声称每帧绘制数据走 push constants。
- 后端 crate 类型不再越过 RHI 边界:下游 crate(render-core、
  render-feature、editor)只导入 `moonfield_rhi` 自己的词汇,
  render-feature 和 editor 都不再直接依赖 `gpu-allocator`。
- `texture_bindless` 补上了其他多设备测试二进制已有的 `DEVICE_LOCK`
  串行化:它的三个测试各自创建 instance + device,在某些 Windows 驱动上
  并发创建会访问越界。

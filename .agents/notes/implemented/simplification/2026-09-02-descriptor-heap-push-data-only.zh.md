# Agent Note: Descriptor heap and push data as the only resource model

Status: implemented

[English](2026-09-02-descriptor-heap-push-data-only.md)

## Problem

RHI 一直同时携带两套资源模型。引擎实际运行的所有图形路径——Core3D 不透明pass 与 egui 后端——早已按 descriptor-heap pipeline（`VK_PIPELINE_CREATE_2_DESCRIPTOR_HEAP_BIT_EXT`、null layout）创建，并通过 `push_data`（`vkCmdPushDataEXT`）送入根数据。而该模型取代的 retained descriptor-set 机制仍以死代码的形式留存：整个 `bind` 模块（`BindGroup`、`BindGroupLayout`、`BindGroupEntry`、`BindingResource`、`BindingType`、`ShaderStage`、`BufferRef`、`Sampler`）、`PipelineOptions::set_layouts`、`CommandBuffer::bind_graphics_descriptor_sets`、`CommandBuffer::push_constants`、`PushConstantRange`/`ShaderStages` 类型，以及只剩一个测试在用的 pipeline 非 heap 分支。compute 根指针路径（`set_bindless_root` 与 `ComputePipeline` 的 push-constant layout）也仍通过 `vkCmdPushConstants` 记录根数据。`core 3d bindless root data` 笔记当时把字面 `push_data` 记为"推迟"；如今其 GPU 侧消费已得到端到端验证。

## Decision

- 整体删除 retained descriptor-set 模型：`bind.rs`（仅保留 `TextureView`，移入 `view.rs`，作为 `Texture`、`OffscreenTarget`、`RenderAttachment` 与 swapchain 使用的借用的 image view 类型）、`set_layouts`、`bind_graphics_descriptor_sets`、`push_constants`、`BufferRef`。
- 所有 graphics pipeline 无条件按 descriptor-heap 模式创建：null layout 加 `DESCRIPTOR_HEAP_EXT` 标志；`PushConstantRange`/`ShaderStages`/`PipelineLayout` 类型不复存在，`PipelineOptions` 只保留 heap mappings。
- compute 路径同样迁移到 push data：`set_bindless_root(input, output)` 通过 `push_data` 写入两个入口指针地址，`ComputePipeline` 与 graphics 对应物一样成为 null layout 的 descriptor-heap pipeline。
- `bindless_graphics_heap_sampling`——唯一仍走非 heap pipeline 的测试——改写为 heap pipeline + push data。

## Alternatives considered

- **保留 retained API 作为未来表面。** 被否：死代码仍须编译、通过 clippy 并接受评审；它许诺的互操作逃生口从未在 descriptor set 上落地。
- **让 compute 路径继续使用 push constants。** 被否：那会让同一存储类存在两套根数据机制；push data 与它别名同一 bank，且已被 `command_push_data` 在 GPU 上验证。

## Consequences

- 单一资源模型：所有 pipeline 都是 descriptor-heap pipeline，根数据一律经 push data 流动。
- RHI 公开 API 缩减——`BindGroup*`、`Sampler`、`ShaderStage(s)`、`PushConstantRange`、`PipelineLayout`、`push_constants`、`set_layouts` 全部移除；需要编译、评审并与驱动保持同步的表面变少。
- `TextureView` 保留为借用的 image view 透传；descriptor heap（`DescriptorHeap`）及其句柄本就是唯一的纹理/采样器来源，保持不变。
- 受支持的驱动集上 headless 测试全部通过；两个 AMD 驱动缺陷测试仍按原样忽略。
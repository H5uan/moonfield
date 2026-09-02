# Agent Note: Shader-to-heap access is untyped only — binding→heap mappings removed

Status: implemented

[English](2026-09-02-remove-binding-heap-mappings.md)

## Problem

Shader 访问 descriptor heap 一直并存两条路径。untyped 路径（`spvDescriptorHeapEXT`——用 `NonUniformResourceIndex` 索引 `ResourceDescriptorHeap[]` / `SamplerDescriptorHeap[]`）本就是 Core3D 不透明 pass、全部 compute pipeline 与多数测试在用的方式。另一条是 binding→heap mapping（`VkDescriptorSetAndBindingMappingEXT` 加 `HEAP_WITH_PUSH_INDEX`）：shader 保留经典 `DescriptorSet`/`Binding` 声明，由驱动按 push data 中读到的槽位索引把变量解析到绑定的 heap 上。生产代码中使用 mapping 的只剩一个消费者——egui 后端——外加一个 `#[ignore]` 的 AMD 驱动缺陷 repro。mapping 机制（`HeapMapping`、`HeapMappingResource`、`PipelineOptions`）存在的唯一目的就是服务这个消费者。

## Decision

- `pipeline.rs` 整体移除 mapping 机制：删除 `HeapMapping`、`HeapMappingResource` 与 `PipelineOptions`，`new_with_options` 不再接收 options 参数（保留其真正在用的多 attachment + depth 形参）。
- `egui.slang` 迁移到 untyped 路径：删除 `[[vk::binding(0, 0)]] Sampler2D` 声明；fragment 阶段直接按根数据中的槽位索引从 heap 取纹理与采样器（`ResourceDescriptorHeap[NonUniformResourceIndex(root.texture)]` + `SamplerDescriptorHeap[NonUniformResourceIndex(root.sampler)]`）。`Sample(uv)` 改为 `Sample(s, uv)`；`GetDimensions`/`Load` 不变。fragment 模块以 `spvDescriptorHeapEXT` capability 编译。
- `graphics_heap_sampling.rs` 中的 mapping repro 改写为 untyped `tex.Load` 变体，保留图形阶段 image-descriptor 的覆盖。两个图形阶段 heap 读取测试（image 与 storage-buffer descriptor）此前因 AMD 26.8.1 驱动缺陷标为 `#[ignore]`，26.9.1 已修复；现在作为驱动回归守卫正常运行。
- 其余 `PipelineOptions::default()` 调用点（Core3D pass、bindless graphics sampling、depth occlusion）全部删除；按 "`binding 0`/`binding 1`" 双数组视角描述 heap 的模块文档更新为 untyped-heap 视角。

## Alternatives considered

- **保留 mapping 作为经典 shader 声明的兼容层。** 被否：它只服务于一个消费者（且该 shader 本就归我们改），让管线创建面翻倍、评审者须与驱动保持同步，还会在 SPIR-V 中产生 `ShaderDescriptorSetAndBindingMappingInfoEXT`——对同样的 bindless 结果而言纯属多余机制。
- **egui 保留 mapping、只删死 API。** 被否：那样 mapping 类型仍会专为 egui 存在，而这恰是本次清理要消灭的单消费者表面。

## Consequences

- RHI 只剩一条 shader→heap 路径：带非均匀槽位索引的 untyped heap 访问。不再产生任何 `DescriptorSetAndBindingMappingEXT` 结构，`PipelineOptions` 不复存在。
- egui 的 fragment shader 现在要求 `spvDescriptorHeapEXT`；vertex 阶段不受影响（它从不访问 heap）。
- 图形阶段 image 读取 repro 以 untyped 形式保留在 `graphics_heap_sampling.rs`；AMD 26.8.1 驱动缺陷测试在 26.9.1 后解除忽略（经 cargo run 与测试本身验证）。
- 已在开发机验证：`egui_headless` 正常渲染，bindless compute 与 graphics 采样往返通过，workspace 测试套件全绿，`-D warnings` 下 clippy 干净，`cargo fmt --check` 干净。
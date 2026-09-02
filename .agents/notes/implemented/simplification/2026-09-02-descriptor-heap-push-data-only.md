# Agent Note: Descriptor heap and push data as the only resource model

Status: implemented

[中文](2026-09-02-descriptor-heap-push-data-only.zh.md)

## Problem

The RHI was carrying two resource models side by side. Every graphics path the
engine actually ran — the Core3D opaque pass and the egui backend — was already
created as a descriptor-heap pipeline (`VK_PIPELINE_CREATE_2_DESCRIPTOR_HEAP_BIT_EXT`,
null layout) that feeds root data through `push_data` (`vkCmdPushDataEXT`). The
retained descriptor-set machinery that this model replaced was still present as
dead surface: the whole `bind` module (`BindGroup`, `BindGroupLayout`,
`BindGroupEntry`, `BindingResource`, `BindingType`, `ShaderStage`, `BufferRef`,
`Sampler`), `PipelineOptions::set_layouts`,
`CommandBuffer::bind_graphics_descriptor_sets`, `CommandBuffer::push_constants`,
the `PushConstantRange`/`ShaderStages` types, and the non-heap branch of pipeline
creation that a single test still exercised. The compute root-pointer path
(`set_bindless_root` and `ComputePipeline`'s push-constant layout) also still
recorded root data through `vkCmdPushConstants`. The `core 3d bindless root
data` note recorded literal `push_data` as deferred; by now its GPU-side
consumption is verified end to end.

## Decision

- The retained descriptor-set model is deleted outright: `bind.rs` (except
  `TextureView`, which moves to `view.rs` as the borrowed image-view type used
  by `Texture`, `OffscreenTarget`, `RenderAttachment`, and the swapchain),
  `set_layouts`, `bind_graphics_descriptor_sets`, `push_constants`, `BufferRef`.
- Every graphics pipeline is created in descriptor-heap mode unconditionally:
  a null layout plus the `DESCRIPTOR_HEAP_EXT` flag; the
  `PushConstantRange`/`ShaderStages`/`PipelineLayout` types are gone,
  `PipelineOptions` now carries only heap mappings.
- The compute path moves to push data too: `set_bindless_root(input, output)`
  writes the two entry-point addresses through `push_data`, and
  `ComputePipeline` is a null-layout descriptor-heap pipeline like its graphics
  counterpart.
- `bindless_graphics_heap_sampling`, the one test still on a non-heap pipeline,
  is rewritten onto the heap pipeline + push data.

## Alternatives considered

- **Keep the retained API as future surface.** Rejected: dead code still must
  compile, pass clippy, and be reviewed; the interop escape hatch it promised
  never materialized on descriptor sets.
- **Leave the compute path on push constants.** Rejected: it would keep two
  root-data mechanisms for one storage class; push data aliases the same bank
  and is GPU-verified by `command_push_data`.

## Consequences

- One resource model: every pipeline is a descriptor-heap pipeline and root
  data always flows through push data.
- The RHI public API shrinks — `BindGroup*`, `Sampler`, `ShaderStage(s)`,
  `PushConstantRange`, `PipelineLayout`, `push_constants`, and `set_layouts`
  are gone; less surface to compile, review, and keep in sync with the driver.
- `TextureView` remains as the borrowed image-view pass-through; the descriptor
  heap (`DescriptorHeap`) and its handles were already the only texture/sampler
  source and stay unchanged.
- Headless tests pass on the covered driver set; the two AMD driver-bug tests
  remain ignored as before.
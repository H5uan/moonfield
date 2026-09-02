# Agent Note: push data GPU consumption and fragment-stage heap sampling

Status: implemented

[中文](2026-09-01-push-data-and-fragment-heap-sampling.zh.md)

## Problem

Two bindless 2.0 verification gaps remained after the Core 3D root-pointer
integration:

1. `vkCmdPushDataEXT` had only a recording-level test
   (`push_data_records_cleanly`); nothing proved that bytes written through
   `push_data` actually reach a shader's push-constant block on the GPU.
2. Heap sampling (`ResourceDescriptorHeap` / `SamplerDescriptorHeap` with no
   descriptor set layout) was verified in compute only; the fragment stage —
   the path material textures will take — had no coverage.

## Decision

Added two headless tests, both passing on the real driver:

- `command_push_data.rs::push_data_feeds_root_pointers`: the plus-one compute
  kernel receives its two root BDA addresses through a single 16-byte
  `cmd_push_data` write (the same layout `set_bindless_root` pushes via
  `cmd_push_constants`); readback asserts out = in + 1. This confirms
  empirically that push data aliases the classic push-constant bank: the
  bound `ComputePipeline` has a classic layout with a 16-byte push range and
  no DESCRIPTOR_HEAP flag, and still consumes push data.
- `bindless_graphics_heap_sampling.rs::fragment_heap_sampling_roundtrip`: a
  graphics pipeline with no descriptor set layout samples the 4x4 red heap
  texture in the fragment shader through
  `ResourceDescriptorHeap[0]` / `SamplerDescriptorHeap[0]` (compiled with the
  `spvDescriptorHeapEXT` capability), multiplies it by a white tint read
  through a `Ptr<float4>` root pointer (one 8-byte FRAGMENT push-constant
  range), and the pixel readback asserts the center of the target is red.

## Alternatives considered

- Requiring `VK_PIPELINE_CREATE_2_DESCRIPTOR_HEAP_BIT_EXT` on the pipeline for
  push-data consumption: the spec aliases push data with push-constant state
  (last setter wins), and the test confirms a classic-layout pipeline
  consumes it — no flag and no RHI change needed.
- A graphics-stage variant of `set_bindless_root`: unnecessary, since
  `CommandBuffer::push_constants` with `ShaderStages::FRAGMENT` already
  covers it, as the Core 3D pass does.

## Consequences

- Both gaps are closed with buffer- and pixel-level evidence; every bindless
  2.0 mechanism the graphics pipeline relies on is now verified end to end.
- The push-data bank is proven to feed shaders on this driver, so per-draw
  root data can move from `cmd_push_constants` to `cmd_push_data` later
  without further RHI work.

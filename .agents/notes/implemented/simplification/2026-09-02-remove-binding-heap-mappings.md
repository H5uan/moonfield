# Agent Note: Shader-to-heap access is untyped only — binding→heap mappings removed

Status: implemented

[中文](2026-09-02-remove-binding-heap-mappings.zh.md)

## Problem

Shader access to the descriptor heaps had two coexisting paths. The untyped
path (`spvDescriptorHeapEXT` — `ResourceDescriptorHeap[]` /
`SamplerDescriptorHeap[]` indexed with `NonUniformResourceIndex`) was already
what the Core3D opaque pass, every compute pipeline, and most tests used. The
other path was the binding→heap mapping
(`VkDescriptorSetAndBindingMappingEXT` with `HEAP_WITH_PUSH_INDEX`), which let
a shader keep classic `DescriptorSet`/`Binding` declarations while the driver
resolved the variable against the bound heaps at a slot index read from push
data. Production code using the mapping was down to exactly one consumer — the
egui backend — plus one `#[ignore]`d AMD driver-bug repro. The mapping
machinery (`HeapMapping`, `HeapMappingResource`, `PipelineOptions`) existed
only to serve that consumer.

## Decision

- `pipeline.rs` loses the mapping machinery outright: `HeapMapping`,
  `HeapMappingResource`, and `PipelineOptions` are deleted, and
  `new_with_options` no longer takes an options argument (it keeps the
  multi-attachment + depth formats it was really used for).
- `egui.slang` moves onto the untyped path: the `[[vk::binding(0, 0)]]
  Sampler2D` declaration is gone; the fragment stage fetches the texture and
  sampler straight out of the heaps at the root-data slot indices
  (`ResourceDescriptorHeap[NonUniformResourceIndex(root.texture)]` +
  `SamplerDescriptorHeap[NonUniformResourceIndex(root.sampler)]`).
  `Sample(uv)` becomes `Sample(s, uv)`; `GetDimensions`/`Load` are unchanged.
  The fragment module compiles with the `spvDescriptorHeapEXT` capability.
- The mapping repro in `graphics_heap_sampling.rs` is rewritten as an untyped
  `tex.Load` variant, keeping the graphics-stage image-descriptor coverage.
  Both graphics-stage heap-read tests — image and storage-buffer descriptor —
  were `#[ignore]`d for the AMD 26.8.1 driver bug, which 26.9.1 fixed; they
  now run unignored as the driver regression guard.
- All remaining `PipelineOptions::default()` call sites (Core3D pass, bindless
  graphics sampling, depth occlusion) are dropped, and the module docs that
  described the two-array `binding 0`/`binding 1` view of the heaps are
  updated to the untyped-heap view.

## Alternatives considered

- **Keep the mapping as a compatibility layer for classic shader
  declarations.** Rejected: it served a single consumer whose shader was
  already ours to edit, doubled the pipeline-creation surface reviewers had to
  keep in sync with the driver, and produced SPIR-V with
  `ShaderDescriptorSetAndBindingMappingInfoEXT` — strictly more machinery for
  the same bindless result.
- **Leave egui on the mapping and delete only the dead API.** Rejected: the
  mapping types would have existed solely for egui, which is exactly the
  one-consumer surface this cleanup was meant to remove.

## Consequences

- The RHI is down to a single shader-to-heap path: untyped heap access with
  non-uniform slot indices. No `DescriptorSetAndBindingMappingEXT` structures
  are ever emitted, and `PipelineOptions` no longer exists.
- egui's fragment shader now requires `spvDescriptorHeapEXT`; the vertex
  stage is untouched (it never accessed the heaps).
- The graphics-stage image-read repro survives in untyped form in
  `graphics_heap_sampling.rs`; the AMD 26.8.1 driver-bug tests were unignored
  after 26.9.1 (verified by cargo run and by the tests themselves).
- Verified on the dev machine: `egui_headless` renders, bindless compute and
  graphics sampling round-trip, the workspace test suite passes, clippy is
  clean with `-D warnings`, and `cargo fmt --check` is clean.
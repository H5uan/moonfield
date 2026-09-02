# Agent Note: shader-reflection-driven pipelines

Status: implemented

[中文](2026-09-01-shader-reflection-driven-pipelines.zh.md)

## Problem

The bindless descriptor-heap pipelines hardcoded the shader pipeline shape
at every level. `GraphicsPipeline::new_with_options` always built exactly
two stages (VERTEX + FRAGMENT) with a literal `"main"` entry point;
`ComputePipeline::new` always built one COMPUTE stage with `"main"`; vertex
layouts (`VertexBufferLayout`) were hand-written in every caller; per-draw
root data was hand-assembled with `bytemuck::bytes_of` against a
comment-synced struct ("Layout must match X in shader"). Nothing verified
any of it at run or compile time — a shader whose `[shader("...")]` or
struct layout changed silently kept the stale host side.

Meanwhile Slang's reflection surfaced exactly the data needed to close this
loop: the stage of each entry point, its root parameters and their byte
placement, and the vertex input layout.

## Decision

Move the shader from "a thing you point a pipeline at" to the single source
of truth, reading everything else off Slang reflection:

- `CompiledShader` now carries the emitted SPIR-V entry-point name (parsed
  from the `OpEntryPoint` — the emitted name is not the source name), so the
  pipeline stops hardcoding `"main"`. `ShaderModule::from_compiled` records
  stage + entry; pipeline construction validates a module's stage against
  the stage list and rejects modules without stage info.
- `GraphicsPipeline::new_with_stages(device, formats, depth, &[ShaderStageDesc], layout)`
  builds a pipeline from an arbitrary stage list (`ShaderStageDesc` is just
  a module); the two-stage constructor is a special case. Mesh/tessellation
  pipelines are longer lists, no new constructors.
- `Compiler` gained `compile_*_with_options` (capabilities + preprocessor
  `macro_define` pairs) and `ShaderCache` (memoized by path/source + entry +
  caps + defines) so shader variants compile once and are shared.
- `Reflection::vertex_layout(entry)` derives the `VertexBufferLayout` from
  the vertex entry's varying inputs (struct fields unwrapped; compact,
  4-aligned packing, matching the existing `PodVertex` convention).
- `Reflection::root_parameters(entry)` + `RootBinder` build the push-data
  blob: `Ptr<T>` roots carry a GPU address, `uniform` roots carry inline
  bytes, all at reflection-reported offsets. `core_3d` and egui record draws
  through it now.
- `Reflection::compute_thread_group_size(entry)` (numthreads), plus
  `struct_rust_source` (emits a `#[repr(C)]` skeleton w/ offsets) and
  `field_user_attributes` as the editor-metadata seam.
- Workspace moved to edition 2024 (enables `if let ... && ...` chains;
  existing nested-`if` sites rewritten to them).

## Alternatives considered

- Using Slang reflection's `name_override()` for the entry name: wrong — it
  reports source-level overrides; the pinned shader-slang-rs rev returns
  `None` for normal named entries, while the emitted SPIR-V is `main`.
- Parsing stage from the emitted SPIR-V execution model instead of
  reflection's `EntryPoint::stage()`: reflection already exposes it; SPIR-V
  parsing is only needed for the entry name.

## Consequences

- The stage/entry/vertex-layout/root-layout contract is now machine-checked
  at pipeline construction: mismatched `[shader(...)]` annotations, missing
  stage info, and drifted struct sizes fail loudly (egui asserts its
  `EguiRoot` size against reflection at pipeline build).
- Shader edits no longer require touching the host code for the common
  shapes (two-stage graphics, single compute); new stages are data.
- GPU pipeline tests still pass unmodified (the two main pipelines now
  construct from reflection), and 5 new RHI unit tests cover cache
  memoization, variant defines, vertex-layout derivation, root blobs, and
  multi-stage (compute+graphics) files.
- Known limitation: `field_user_attributes` returns empty with the pinned
  shader-slang-rs rev on SPIR-V targets (verified by probe); the API shape
  is kept and asserted loosely so a Slang upgrade surfaces it.
# Agent Note: Access-scoped bindless barriers, and the sync1 path removed

Status: implemented

[中文](2026-09-12-access-scoped-barriers-and-sync2-only.zh.md)

## Problem

`CommandBuffer::barrier` took a stage pair plus a `BarrierHazard` enum
(`Memory` / `Descriptors`) and always emitted the widest access masks —
`MEMORY_READ | MEMORY_WRITE` on both sides, with `Descriptors` only adding
`SHADER_SAMPLED_READ` to the destination. Callers could not say what a
barrier actually ordered, so the render-core door switches recorded
`ALL → ALL` stage pairs as accepted over-synchronization. Separately the
crate kept a legacy sync1 `pipeline_barrier` (`vkCmdPipelineBarrier`) solely
for image-layout transitions in the upload/offscreen paths — a second
barrier API with weaker masks beside the sync2 one.

## Decision

- `sync.rs` gains an `Access` newtype over `vk::AccessFlags2`, mirroring
  `Stage` (associated consts, `|` combination): `NONE`,
  `INDIRECT_COMMAND_READ`, `SHADER_READ`, `SHADER_WRITE`,
  `SHADER_SAMPLED_READ`, `COLOR_ATTACHMENT_READ`/`COLOR_ATTACHMENT_WRITE`,
  `DEPTH_STENCIL_READ`/`DEPTH_STENCIL_WRITE`, `TRANSFER_READ`/
  `TRANSFER_WRITE`, `MEMORY_READ`/`MEMORY_WRITE`, and `ALL`
  (`MEMORY_READ | MEMORY_WRITE`). `Stage` gains `ALL_GRAPHICS`.
- `CommandBuffer::barrier(before, before_access, after, after_access)` —
  four positional scopes — emits the same single global `MemoryBarrier2`
  with real access masks. `BarrierHazard` is deleted; its `Descriptors`
  case lives on as `SHADER_SAMPLED_READ` in the destination access of every
  call site whose consumer samples the descriptor heap (`SHADER_READ`
  subsumes it in Vulkan's access hierarchy; the sites name it explicitly).
- The render-core doors record scoped transitions: compute→rendering is
  `(COMPUTE, SHADER_WRITE) → (VERTEX | FRAGMENT, SHADER_READ |
  SHADER_SAMPLED_READ)` (vertex pulling plus heap sampling);
  rendering→compute is `(ALL_GRAPHICS, COLOR_ATTACHMENT_WRITE |
  DEPTH_STENCIL_WRITE | SHADER_WRITE) → (COMPUTE, SHADER_READ |
  SHADER_SAMPLED_READ | SHADER_WRITE)`; rendering→rendering pairs that
  producer with `(ALL_GRAPHICS, attachment reads and writes | SHADER_READ |
  SHADER_SAMPLED_READ)`; dispatch chains are `(COMPUTE, SHADER_WRITE) →
  (COMPUTE, SHADER_READ | SHADER_WRITE)`. `ALL_GRAPHICS` is the deliberate
  stage widening for raster passes: `Stage` carries no fragment-test or
  attachment constants, and the access masks stay honest.
- The sync1 path is gone. Image layout transitions (uploader
  init/upload, offscreen init/readback) emit sync2
  `vk::ImageMemoryBarrier2` through the crate-internal
  `CommandBuffer::image_barriers`; `pipeline_barrier` is deleted. No
  `vkCmdPipelineBarrier` remains in the crate.
- Texture-init transitions stay per-transition rather than batched: the
  uploader interleaves each transition with its copy, and a device-held
  pending list drained at command-buffer begin would redesign the
  uploader/offscreen flow to save one barrier per texture.

## Alternatives considered

- **Two scope structs (`Scope { stage, access }`) instead of four
  positional args.** Two new types to name one call shape; the four-arg
  form mirrors the reference API and keeps call sites flat.
- **Batched `UNDEFINED → GENERAL` initialization list on the device.**
  Batching saves one barrier per texture but forces a pending list shared
  across the uploader and offscreen paths; keeping each transition next to
  its copy keeps recording linear.
- **Finer raster stages (`COLOR_ATTACHMENT_OUTPUT`, the fragment-test
  stages) instead of `ALL_GRAPHICS`.** More constants to pair with access
  masks correctly for no behavioral gain — the access masks already carry
  the honest hazard, and `ALL_GRAPHICS` still excludes compute and
  transfer.

## Consequences

- Every barrier call site spells its hazard; a wrong stage/access pairing
  fails sync2 validation instead of silently over-synchronizing.
- Render-core inserts no `ALL → ALL` barriers; raster-involving switches
  widen stages to `ALL_GRAPHICS` by design, documented at the doors.
- The crate has one barrier entry point per shape, both sync2: the global
  memory `barrier` (public) and `image_barriers` (crate-internal).
- The bindless-barrier GPU tests exercise access-mask variants (shader
  read/write, sampled read) instead of the deleted hazard enum.

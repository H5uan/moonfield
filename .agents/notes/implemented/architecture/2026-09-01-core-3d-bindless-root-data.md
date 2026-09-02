# Agent Note: core 3d pass bindless root data

Status: implemented

[中文](2026-09-01-core-3d-bindless-root-data.zh.md)

## Problem

The Core3D opaque pass pushed the whole per-draw payload — an 80-byte block
of mvp + color — inline through `vkCmdPushConstants`. That left the graphics
pipeline on the retained push-constant model while the RHI's bindless 2.0
foundation (descriptor heap, `GpuBumpAllocator`, root pointers) had already
proved itself on the compute path: Slang lowers shader entry `Ptr<T>`
parameters into a push-constant block holding a device address, so draw data
can live in GPU memory and only the pointer travels through the command
buffer.

## Decision

- `ScenePushConstants` (mvp + color, 80 B) becomes `DrawData`, stored per
  draw in a new render-world resource `FrameDrawArena`.
- `FrameDrawArena` mirrors `FrameUploader`'s ring: `MAX_FRAMES_IN_FLIGHT`
  `GpuBumpAllocator`s; `begin_frame(slot)` calls `free_all` on the slot after
  `acquire_window_frames`'s timeline wait proves that slot's previous GPU work
  finished; `alloc_draw_data` carves a `DrawData` from the current slot. The
  interior `Mutex` matches the `DescriptorHeap` precedent so draw functions,
  which receive only `&World`, can still allocate.
- The pipeline is created in descriptor-heap mode — null layout, no push
  ranges — and the root is a single `GpuPtr` (`ROOT_POINTER_SIZE`, 8 B)
  delivered through push data (`push_data`); `DrawMesh` writes `DrawData`
  through the bump host pointer and pushes the device address as the root.
  Vertex and index buffers keep their classic binding — this milestone changes
  root data only.
- Shaders declare `Ptr<DrawData> root` as an entry parameter and read
  `root[0].mvp` / `root[0].color`; the matrix stays `column_major` with
  `to_cols_array()`, so the bytes reaching the GPU are unchanged.
- `Stage::VERTEX` / `Stage::FRAGMENT` join `bindless.rs` so graphics-stage
  bindless barriers can express the same pointer model.

## Alternatives considered

- Literal `push_data` (the descriptor-heap root bank) instead of a root
  pointer in push constants: at the time, the GPU-side consumer of
  `vkCmdPushDataEXT` was unwired and Slang's entry `Ptr` lowering was verified
  for push constants only, so the pointer-through-push-constant form shipped
  first. Once `command_push_data` verified GPU-side consumption, the
  [push-data-only cleanup](../simplification/2026-09-02-descriptor-heap-push-data-only.md)
  moved every pipeline onto push data and deleted the retained model.
- Keeping the 80-byte payload inline in push constants: leaves the pass on
  the retained model; nothing about the bindless pipeline changes.

## Consequences

- The graphics pass is bindless-shaped: per-draw root data is a single GPU
  pointer, payloads live in reused GPU memory, and textures/materials later
  plug into the same root struct.
- Root data shrinks from an 80 B inline push to a single device address
  through the descriptor-heap push-data bank — no set layouts, no push-constant
  ranges anywhere in the pipeline.
- The draw arena's frame pacing rides the window frame timeline; the
  single-window slot assumption is documented for the multi-window future.
- Tests: `test_opaque_pass_draws_mesh` and `test_opaque_pass_depth_occludes`
  stay green unchanged — they now exercise the pointer path; headless tests
  drive arena slot 0 manually since no window frame loop runs there.
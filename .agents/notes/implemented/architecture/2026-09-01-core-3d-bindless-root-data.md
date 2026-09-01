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
- The pipeline's push-constant range shrinks from 80 B to 8 B
  (`ROOT_POINTER_SIZE`, one `GpuPtr`); `DrawMesh` writes `DrawData` through
  the bump host pointer and pushes the device address as the root. Vertex and
  index buffers keep their classic binding — this milestone changes root data
  only.
- Shaders declare `Ptr<DrawData> root` as an entry parameter and read
  `root[0].mvp` / `root[0].color`; the matrix stays `column_major` with
  `to_cols_array()`, so the bytes reaching the GPU are unchanged.
- `Stage::VERTEX` / `Stage::FRAGMENT` join `bindless.rs` so graphics-stage
  bindless barriers can express the same pointer model.

## Alternatives considered

- Literal `push_data` (the descriptor-heap root bank) instead of a root
  pointer in push constants: the GPU-side consumer of `vkCmdPushDataEXT` is
  not wired — `command_push_data` only verifies recording, and Slang's entry
  `Ptr` parameter lowering is verified for push constants only. Adopting it
  would require unproven RHI/shader work first, so it is deferred.
- Keeping the 80-byte payload inline in push constants: leaves the pass on
  the retained model; nothing about the bindless pipeline changes.

## Consequences

- The graphics pass is bindless-shaped: per-draw root data is a single GPU
  pointer, payloads live in reused GPU memory, and textures/materials later
  plug into the same root struct.
- Push-constant usage drops from 80 B to 8 B per draw on a pure-heap
  pipeline (no descriptor set layouts).
- The draw arena's frame pacing rides the window frame timeline; the
  single-window slot assumption is documented for the multi-window future.
- Tests: `test_opaque_pass_draws_mesh` and `test_opaque_pass_depth_occludes`
  stay green unchanged — they now exercise the pointer path; headless tests
  drive arena slot 0 manually since no window frame loop runs there.
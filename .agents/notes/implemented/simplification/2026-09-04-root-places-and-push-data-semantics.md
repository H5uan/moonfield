# Agent Note: Per-draw root encoding without allocation, and push-data semantics corrected

Status: implemented

[中文](2026-09-04-root-places-and-push-data-semantics.zh.md)

## Problem

Every draw cloned the pipeline's `RootBinder` — two `Vec`s with `String`
names plus a linear search — to write 8 bytes (core 3D) or 24 bytes
(egui); egui's 24 bytes included 16 frame-constant bytes re-pushed for
every mesh. `Core3dFrame` and `RenderTargetSizes` were deep-cloned every
frame to dodge `World` borrows that interior mutability already permits.
`Texture::bindless` hardcoded 4 bytes per pixel. And the crate's push-data
docs claimed push data "aliases the push-constant bank" — a claim the
extension spec does not make.

## Decision

- `RootBinder::pointer_param`/`uniform_param` resolve a
  `RootParamPlace` (offset, size, kind) once at pipeline build. A draw
  encodes pointer roots on the stack (`RootParamPlace::pointer_bytes`)
  and pushes them at the place's offset — no allocation, no name lookup.
- `EguiRoot`'s varying fields (texture, sampler) are the struct's tail;
  the pass pushes the 16-byte static prefix once and each draw pushes the
  8-byte tail at its offset. The tail position and the struct size are
  guarded at pipeline build.
- `Core3dFrame` and `RenderTargetSizes` are borrowed, not cloned —
  `Ref`/`RefMut` of distinct resources coexist.
- Push-data docs state the spec's terms: push data is the descriptor-heap
  pipelines' root-data interface, read through the existing
  `PushConstant` storage class; push constants rely on set layout state
  and are incompatible with heap pipelines (the two command families
  invalidate each other on a command buffer). Persistence of bytes
  outside a written range is not explicitly specified —
  `push_data_ranges_persist_across_writes` GPU-verifies it (three ranges,
  the uniform written last, one dispatch reads all).
- `Texture::bindless` sizes its upload check by
  `Format::bytes_per_pixel`.

## Alternatives considered

- **Cloning the root blob once per pass instead of per draw.** Still one
  allocation and one copy per draw; place resolution removes the last
  one.
- **Pushing the full `EguiRoot` per draw.** Three times the bytes, and
  frame constants re-pushed hundreds of times per frame.

## Consequences

- `DrawMesh`'s per-draw root work is one stack encode plus one
  `push_data`; egui's is 8 bytes and a scissor. `set_pointer`/`set_bytes`
  remain the one-shot blob API (tests); places are the hot-path API.
- The static-prefix pattern rests on bank persistence, which the spec
  does not spell out — the GPU test is the guard, and a driver that
  clears unoverwritten bytes fails it loudly.

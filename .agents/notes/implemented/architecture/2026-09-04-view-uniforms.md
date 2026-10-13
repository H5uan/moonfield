# Agent Note: ViewUniforms — per-view data has one home

Status: implemented

[中文](2026-09-04-view-uniforms.zh.md)

## Problem

No per-frame or per-view data reached the GPU. The view-projection was
folded into every draw's MVP on the CPU — one matrix multiply per item
per view, then copied through the queue item, the frame snapshot, and the
arena record — and the fragment shader's light direction was a hardcoded
constant. Lights, time, and fog had nowhere to live, and GPU culling (the
splat roadmap) needs the view-projection on the GPU side.

## Decision

- `ViewUniforms { view_proj, view_pos }` is the per-view record: one per
  pass, written into the frame draw arena, its address pushed once per
  pass through the reflected `Ptr<ViewUniforms>` placement. Lights, time,
  and fog grow in this struct.
- `DrawData` shrinks to `{ model, color }` — per-draw data describes the
  object, not the camera. The queue stores the model matrix without
  multiplying; the pass computes `view_proj` from the target's real
  extent (the queue no longer reads `RenderTargetSizes`).
- The vertex shader composes `view_proj * model * position` on the GPU.
  The core 3D pixel tests pass unchanged — the multiply moved, the result
  did not.

## Alternatives considered

- **Keeping the CPU-side MVP until the GPU-driven refactor.** Leaves
  lights and time homeless and blocks GPU culling; the change is
  independent of vertex pulling and lands on its own.
- **Inline view uniforms through push data.** 80 bytes per pass against
  8: the arena record costs nothing per draw and keeps the push-data
  budget for per-draw data.

## Consequences

- Per-draw CPU cost drops by one 4×4 multiply per item per view, and
  `RenderTargetSizes` is only read where targets are ensured.
- The `Ptr<T>` pointee layout is Slang's natural (C-like) layout — the
  emitted SPIR-V names pointee types `..._natural` (offsets baked into
  pointer arithmetic) while entry-parameter blocks are
  `EntryPointParams_std430`. The Rust mirror is a plain `#[repr(C)]`
  field-for-field match; `two_pointer_roots_and_ptr_struct_layout` pins
  the offsets and the two-pointer root shape.

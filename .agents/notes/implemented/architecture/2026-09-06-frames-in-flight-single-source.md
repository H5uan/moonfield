# Agent Note: single frames-in-flight constant owned by the RHI

Status: implemented

[中文](2026-09-06-frames-in-flight-single-source.zh.md)

## Problem

The retirement ring depth (`moonfield_rhi::RETIRE_RING`) and the frame loop's
frames-in-flight (`moonfield_render_core::MAX_FRAMES_IN_FLIGHT`) must be
equal, or a retired slot could be drained before its submission completed.
The two were separate `usize = 2` constants kept in agreement by a runtime
`assert_eq!` in `RenderPlugin::build` — a compile-time fact checked at
runtime, and a third consumer (the editor's `EguiFrameResources`) took the
count as a parameter trusting its caller.

## Decision

`moonfield_rhi::RETIRE_RING` is the single source of truth.
`moonfield_render_core::MAX_FRAMES_IN_FLIGHT` becomes a const alias of it, so
downstream names (`render-feature`'s `FrameDrawArena`, the editor's
`EguiFrameResources::new(device, MAX_FRAMES_IN_FLIGHT)`) keep working. The
`assert_eq!` in `RenderPlugin::build` is deleted — the equality is now
definitional. The RHI owns the value because the retirement ring is the
constraint the frame loop must respect, and the RHI cannot depend on
render-core.

## Alternatives considered

- **Render-core owns the value; the RHI takes the depth as a `Device`
  parameter.** Rejected: constructor plumbing for what is a fixed policy of
  the frame loop, and every test or tool building a `Device` directly would
  have to know the right number.
- **A shared constants crate.** Rejected: a crate for one constant.

## Consequences

- Changing the depth is one edit in `moonfield-rhi`; the mismatch class is
  eliminated at compile time.
- `RETIRE_RING`'s doc comment now states the aliasing direction so future
  readers know which name is authoritative.

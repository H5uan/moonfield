# Agent Note: Render recording context

Status: implemented

[中文](2026-09-09-render-recording-context.zh.md)

## Problem

The [redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) specified a closed recording surface — `RenderContext` with three typed doors and a stage-state-machine for barriers — but M0–M5 landed the schedule skeleton without it. Every recording system pulled the raw frame `CommandBuffer` through `FrameContext::current_command_buffer()`: the opaque pass, the orphan-target clear, the egui pass, and the splat sort all bypassed the intended doors, and the compute door did not exist at all. `TrackedRenderPass` was a local wrapper inside `record_view_pass` that deduplicated only pipeline binds, keyed by the Rust address of the pipeline — sound only by the convention that pipelines rebuild in `PrepareViews`, and blind to viewport, depth, and cull state. Cross-pass synchronization was schedule order plus per-pass hardcoded layout values; the rhi's `CommandBuffer::barrier` had no render-layer caller.

## Decision

- `render-core` gains a `context` module. `RenderContext` is a `SystemParam` (and directly constructible inside exclusive systems via `RenderContext::get(world)`) that holds the frame's `Ref<FrameContext>` and a `RefMut<RecordingState>`; both doors read as absent when the resources are missing, so passes no-op headless and outside the frame. Its three doors: `begin_rendering(&RenderPassDesc) -> TrackedRenderPass`, `compute() -> ComputeRecording`, and `barrier(before, after, hazard)`.
- `RecordingState` is a render-world resource carrying the machine's phase (`Idle → Rendering/Compute`). `acquire_window_frames` inserts a fresh one when the frame begins; `submit_window_frames` drops it. Door switches insert the rhi's global, resource-less memory barriers automatically: `COMPUTE → COMPUTE` between compute phases, and the broadest `ALL → ALL` pair wherever raster output is (or may be) involved — `Stage` has no `COLOR_ATTACHMENT_OUTPUT` constant, so the conservative pair is the only expressible one. A manual `barrier` records as given and resets the phase to `Idle`, marking the hazard handled.
- `TrackedRenderPass` moved into the `context` module and is what `begin_rendering` returns. Its bind tracking keys the pipeline's raw Vulkan handle through the new `GraphicsPipeline::id()` — unique among live pipelines — and now also deduplicates viewport, depth-state, and cull-state sets; tracking resets at `begin_rendering`. Scissor and blend are untracked passthroughs (UI passes change them per draw).
- `ComputeRecording` wraps the compute door: `bind_pipeline`, root-data pushes, and `dispatch`, with an automatic `COMPUTE → COMPUTE` memory barrier before every dispatch after the first — the read-after-write chain every compute pass has. `RadixSort::record` takes it and its three hand-written per-pass barriers are deleted; the machine emits the same set.
- `record_view_pass` records through the raster door and splits its body into `record_view_items` (dynamic states, view uniforms, item dispatch) so tests that own their command buffer drive the same path. The egui pass, orphan-target clear, and splat sort all go through the doors; `FrameContext::current_command_buffer` is crate-private.

## Alternatives considered

- **`RenderContext` owning a `CommandBuffer` clone.** Lost: `CommandBuffer::drop` frees the Vulkan command buffer from its pool, so a clone double-frees; the `Ref`/`RefMut` pair borrows the existing per-slot buffer with no unsafe.
- **Minimal stage pairs for raster switches** (`FRAGMENT → COMPUTE` etc.). Lost: raster writes happen at attachment output, which `Stage` cannot name; under-naming risks a real hazard in the composite ordering, so the broad pair is the honest choice. Revisit if `Stage` grows an attachment-output constant.
- **Automatic barriers only between compute dispatches that touch the same buffers.** Lost: the rhi barrier is deliberately resource-less (bindless pointers), so per-resource tracking is not expressible; the conservative global barrier is the [redesign's stated form](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md).

## Consequences

- No render-layer system names `CommandBuffer` anymore; the rhi's raw surface is reachable only through the three doors or by owning a command buffer (tests).
- Every raster→raster and compute→raster switch now carries a global barrier — over-synchronization for clears, correct-by-construction for read-after-write chains (the GS composite lands on this).
- `begin_frame_draw_arena`, `FrameContext` checks, and per-pass `TrackedRenderPass::new` boilerplate disappear from the pass systems; a new pass opens a door and records.

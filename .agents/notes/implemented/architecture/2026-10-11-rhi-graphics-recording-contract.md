# Agent Note: RHI graphics recording contract

Status: implemented

[中文](2026-10-11-rhi-graphics-recording-contract.zh.md)

## Problem

The [contract narrowing](2026-10-11-rhi-sync-memory-contract-narrowing.md)
unified memory, sync, and compute recording across the backends; the raster
(graphics) surface still diverged:

- `GraphicsPipeline::new` differed in arity and parameter order — Vulkan
  `(device, color_format, vertex_shader, fragment_shader)`, Metal
  `(device, module, vertex_entry, fragment_entry, format)`.
- `RenderPassDesc` was not the same type: Vulkan carried `layer_count`,
  Metal did not, and Metal silently ignored `depth_attachment`.
- Metal's rebuilt render encoder did not replay the viewport (a recorded
  viewport was lost across encoder rebuilds).
- `TextureView` accessor parity was broken: Metal exposed
  `width`/`height`/`format`, Vulkan exposed none.

These were the items keeping the raster consumers (the `Core3d` pass,
egui) from compiling against the Metal backend — the prerequisite for the
[Metal proposal](../../rejected/architecture/2026-10-09-metal-4-backend.md)'s
editor phase. Everything they need beyond this surface (offscreen/depth
targets, the egui Metal backend, timestamps) is owned by that proposal and
the [layering proposal](../../implemented/architecture/2026-10-10-rhi-interface-and-backend-layering.md),
not duplicated here.

## Decision

- **Pipeline constructors.** `GraphicsPipeline::new(device, color_format,
  vertex, fragment)` and `new_with_options(device, color_formats,
  depth_format, vertex, fragment)` on both backends, entries carried by the
  `ShaderModule` (`from_compiled` records the emitted name, `from_msl`
  names its entry); construction fails `Validation` when a module carries
  no entry. Metal's explicit-entry constructors are gone.
- **`RenderPassDesc` parity.** `layer_count` left the public shape: the
  consumer audit found every caller rendering layer 1 — no layered-rendering
  consumer exists — so both backends render single-layer and no field is
  silently ignored. Metal honors `depth_attachment`: the render encoder
  gains the depth target (texture, load/store/clear), and pipelines declare
  the depth format through `new_with_options` — Metal validates the
  pipeline/pass pair at draw time. `RenderAttachment` is `Clone` on both
  backends.
- **Depth state.** Metal's `set_depth_state` maps onto a per-device cached
  `MTLDepthStencilState`; a write with the test disabled is forced off
  (Vulkan's depth writes only happen with the test enabled, and Metal has
  no disable switch once a state is set).
- **Encoder-rebuild state replay.** Metal's render-encoder rebuild (a
  `push_data` after encoded commands) replays the recorded dynamic state:
  the argument table, viewport and scissor (the render area, or the
  recorded override), cull, depth state, and the bound pipeline. A fresh
  encoder carries no state, and per-pass overrides reset at
  `begin_rendering`.
- **Store ops under rebuilds.** Metal bakes load/store at encoder creation;
  an intermediate encoder must not discard what later encoders reload.
  Every render encoder the Metal backend creates stores; `end_rendering`
  applies a declared `Discard` through a trailing no-op encoder (load +
  declared store), so the declared op survives.
- **`MTL4RenderPassDescriptor` depth attachment.** `setDepthAttachment`
  copies its argument; the descriptor is built and fully configured before
  the copy. Mutating the getter's return is lost.
- **`TextureView` accessors.** `width`/`height`/`format` on both backends;
  Vulkan's view carries the extent and format from its owner (`Texture`,
  `OffscreenTarget`, `DepthBuffer`, `Swapchain`).
- **Raster target carrier.** `Texture::new_render_target` and
  `read_pixels(&self, device)` on both backends: a renderable color target
  with CPU readback, for tests and editor viewports. Vulkan's target
  transitions to `GENERAL` through a one-time internal submission at
  creation (renderable on return); readback copies through a `CpuReadOnly`
  staging allocation.
- **The raster contract test** (`tests/contract_raster.rs`) pins the
  surface through the facade's public API, one spelling: Slang vertex-pull
  + fragment, `push_data` roots with `RootBinder`-derived placement,
  clear/draw/readback. Its assertions separate "rendered" from "cleared"
  (black clear, red triangle), so a vacuous pass is impossible.
- **Capability flags.** None flip: the four `Capabilities` flags are all
  owned by the Metal proposal's phases; the raster parity items are
  unconditional API surface, not optional device capabilities.

## Alternatives considered

- **Keep explicit-entry pipeline constructors on Metal.** Lost: the
  module-carried entry removes the mislabeling door, and two spellings of
  pipeline construction is exactly the drift this contract removes.
- **Implement layered rendering on Metal instead of dropping
  `layer_count`.** Lost: no consumer renders layers; removal is the
  cheaper parity (the audit decides, and the field left the shape).
- **A per-backend-gated raster contract test.** Lost for the same reason
  as in the compute contract: a gated test pins nothing on the second
  backend; one spelling is the point.
- **Fold graphics into the Metal proposal's phases.** Lost: that proposal
  owns backend capability (what Metal can do); this one owns the public
  contract shape (what consumers spell). Keeping them separate lets the
  contract land independently of the macOS bring-up.

## Consequences

- The `Core3d` pass and egui spell only shared raster names
  (`new_with_options`, `set_depth_state`, `push_data`, the depth-carrying
  `RenderPassDesc`); their remaining Metal-path gaps (`OffscreenTarget`,
  `DepthBuffer`, `FrameUploader`, egui's Vulkan backend) are the Metal
  proposal's phases.
- Slang graphics entries read their `Ptr<T>` root through the
  `EntryPointParams` blob at argument-table slot 0 on Metal — the same
  shape as compute, so `push_data` serves both stages on both backends
  (the [Metal 4 skeleton note](2026-10-09-metal-4-skeleton.md) records the
  binding model).
- The Metal gpu tests construct one module per entry point and deliver
  roots through `push_data`; the hand-written-MSL tests keep their
  `[[buffer(0)]]`-declared buffers bound through the crate-internal
  `set_buffer`.
- The raster contract runs on Metal on Apple targets and on Vulkan where a
  device carries the RHI's required extensions; elsewhere it compiles and
  skips (a hardware skip is not evidence of equivalence — the same rule as
  the compute contract).
- The first macOS run (Apple Silicon) validated the Metal halves of both
  the [sync/memory contract](2026-10-11-rhi-sync-memory-contract-narrowing.md)
  and this contract: the full gpu-test and contract suites pass, including
  the compute contract's pending-barrier replay.

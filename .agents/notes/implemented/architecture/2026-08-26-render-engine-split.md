# Agent Note: Engine layer split out of the Vulkan RHI

Status: implemented

[中文](2026-08-26-render-engine-split.zh.md)

## Problem

The RHI crate played two roles in one crate. As Lunar Mare it owned the
ash-based backend (`vulkan/*`, `types`, `bind`, `indirect`); as an engine
layer it also owned extraction (`extract.rs`: `MainEntity`,
`extract_cameras`, `extract_with_transform`), camera snapshots (`scene.rs`:
`ExtractedView`, `ViewTarget`), the `RenderPlugin`, and the window frame-loop
systems (`vulkan/window.rs`: `extract_windows`, `create_window_surfaces`,
`acquire_window_frames`, `submit_window_frames`). The crate therefore depended
on `moonfield-app`, `moonfield-ecs`, `moonfield-camera`, and
`moonfield-window`, violating the pure backend surface promised by
[Vulkan RHI boundary](2026-08-19-vulkan-rhi-boundary.md).
[Renderer aligned with Bevy](2026-08-24-renderer-bevy-alignment.md) recorded
the old home of `ExtractedView`, `ViewTarget`, and the extraction systems
inside the RHI crate.

## Decision

The RHI crate is renamed `moonfield-rhi` (Lunar Mare) and keeps only the RHI
surface: the `vulkan/*` resource and command code (device, instance, buffer,
texture, offscreen, pipeline, shader, sync, swapchain, bindless), `types.rs`,
`bind.rs`, `indirect.rs`, and the `RenderDevice` resource type. It drops the
`moonfield-app`, `moonfield-ecs`, `moonfield-camera`, and `moonfield-window`
dependencies and remains the only crate linking `ash`. The frame loop's submit
and present details live in the RHI as vocabulary helpers
(`Device::submit_frame`, `Device::wait_idle`, `Swapchain::format_srgb`,
`Swapchain::acquire_next_image`/`queue_present` returning
`Error::SurfaceOutOfDate`), so the engine layer links no `ash` of its own.

A new crate, `moonfield-render-core` (Selene), owns the engine layer:
`extract.rs` (`MainEntity`, `extract_cameras`, `extract_with_transform`),
`scene.rs` (`ExtractedView`, `ViewTarget`, and the `ViewTargets` attachment map
relocated from `moonfield-render-feature/src/core_3d/pass.rs`),
`window.rs` (the window frame loop: `extract_windows`,
`create_window_surfaces`, `acquire_window_frames`, `submit_window_frames`,
`ExtractedWindow`, `WindowSurfaces`, `WindowFrameDemand`, `WindowSurfaceData`,
`MAX_FRAMES_IN_FLIGHT`), and `plugin.rs` (`RenderPlugin`, which creates the
`RenderDevice` via `RenderDevice::new` and inserts it into the render world at
the same point, preserving LIFO teardown).

Consumers follow the new layout: `moonfield-render-feature` (Lunaris) orders
its systems against Selene's `acquire_window_frames`/`submit_window_frames`
and takes `extract_with_transform`, `ExtractedView`, `ViewTarget`, and
`ViewTargets` from Selene; `moonfield-editor` takes `RenderPlugin`, the frame
loop, `WindowSurfaces`, `WindowFrameDemand`, `MAX_FRAMES_IN_FLIGHT`, and
`ViewTargets` from Selene while `egui_vk` keeps using the RHI's pure resources
(`Buffer`, `BindGroup`, ...). The codenames Lunar Mare (RHI), Selene (engine),
and Lunaris (features) are declared in the respective READMEs.

## Alternatives considered

**Engine layer into `moonfield-app`.** Rejected: `moonfield-app` is the plugin
framework whose `Render`/`RenderPrepare`/`RenderQueue` labels are renderer-
agnostic; inheriting the engine layer would make the app depend on the RHI.

**Engine layer into `moonfield-render-feature`.** Rejected: that crate is the
feature layer and already hosted the other half of the engine (`core_3d`,
`render_phase`); merging the engine into it preserved the conflation this split
removes.

**Document-only boundary.** Rejected: the acceptance criterion is a structural
property — `moonfield-rhi` compiles without the engine dependencies — and prose
does not enforce that.

**Keep `moonfield-render` as the RHI crate name.** Rejected: with two
render-family crates, `moonfield-render` reads as "the renderer" (the engine),
not the RHI; `moonfield-rhi` states the crate's role precisely. The rename is
mechanical.

**`RenderDevice` into Selene.** Rejected: it is a plain resource type with no
ECS dependency; keeping it in the RHI lets headless one-shot consumers
(`RenderDevice::new` in tests) stay RHI-only.

## Consequences

- The RHI boundary is structural: `moonfield-rhi` compiles without
  `moonfield-app`/`moonfield-ecs`/`moonfield-camera`/`moonfield-window`, and
  any crate linking `ash` outside `moonfield-rhi` breaks the workspace rule.
- The window frame loop's submit, present, and format mapping are RHI
  vocabulary helpers, so Selene links no `ash`.
- `RenderDevice` insertion order is unchanged, preserving the render world's
  LIFO resource teardown.
- `ViewTargets` exposes `iter()` and `ensure()`, so the feature crate no
  longer needs private-field access to the attachment map.
- Windowed behavior is unchanged: the editor and feature crates build against
  the same systems and ordering anchors under new names; `cargo test
  --workspace` and the headless triangle smoke test pass.
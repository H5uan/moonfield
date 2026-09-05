# Agent Note: Primary-window 3D pass — cameras can draw straight to the window

Status: implemented

[中文](2026-09-05-primary-window-3d-pass.zh.md)

## Problem

`RenderTarget::PrimaryWindow` existed in the camera vocabulary, but nothing
drew there: `prepare_view_targets` only created offscreen attachments for
`Viewport` views, and `main_opaque_pass_3d` only recorded into
`ViewTargets` — a game-path camera (no editor) presented nothing. The window
frame loop acquired swapchain images nobody rendered into.

## Decision

The window path mirrors the offscreen one at the pass level, with the surface
supplying attachments:

- **rhi**: a first-class `DepthBuffer` (standalone `D32Sfloat` attachment with
  ring-deferred teardown) in `offscreen.rs`, reused helpers included.
- **render-core**: `WindowSurfaceData` owns a `DepthBuffer` sized to the
  swapchain (created in `new`, resized in `recreate`), exposed via
  `depth_view()`. `extract_cameras` writes the base `WindowFrameDemand` — any
  camera targeting `PrimaryWindow` demands frames.
- **editor**: `extract_editor_frame` ORs its UI demand into the existing value
  instead of overwriting (extract systems run in registration order;
  `RenderPlugin` registers first).
- **render-feature**: `record_view_pass` now takes a `PassTarget` (color/depth
  views, extent, final color layout) instead of `&OffscreenTarget`, and
  `main_opaque_pass_3d` gained a second loop: a primary view targeting
  `PrimaryWindow` records into each in-progress surface's swapchain image
  (final layout `Present`), depth-tested against the surface's depth buffer.
  Offscreen recording is unchanged.

Known limits, deliberately accepted: the pass is format-locked to
`VIEW_TARGET_FORMAT` (a swapchain negotiated to another format, e.g. sRGB, is
skipped with `error_once!` until pipelines become format-keyed); the
`RenderTarget` default stays `Viewport`, so window rendering is opt-in via
`CameraTarget`; and a window-targeted camera combined with the editor's UI
pass (which clears the swapchain image) is not a supported composition.

## Alternatives considered

- **Render the scene to an offscreen target and blit to the swapchain.**
  Rejected: an extra full-screen copy per frame and a second image in flight,
  to avoid writing one attachment branch; dynamic rendering makes the direct
  path a layout difference, not a new pipeline.
- **Flip the `RenderTarget` default to `PrimaryWindow`.** Deferred: it changes
  what scene-loaded and editor-spawned cameras mean (the editor relies on the
  `Viewport` default), so it is a product decision, not part of closing the
  loop.
- **Format-keyed pipelines (`HashMap<Format, Core3dPipeline>`) now.**
  Deferred: `Swapchain::new` already prefers `B8G8R8A8_UNORM`, so the single
  pipeline serves every supported surface today; the map is cheap to add when
  an sRGB-only target actually appears.

## Consequences

- A camera with `CameraTarget(RenderTarget::PrimaryWindow)` renders depth-
  tested meshes straight into the window — the game path works without the
  editor.
- `WindowFrameDemand` is OR-accumulated: camera-driven and editor-driven
  demand compose instead of clobbering each other.
- `record_view_pass` is target-agnostic (`PassTarget`); the same code serves
  offscreen and swapchain attachments.
- Tests: `extract_cameras` demand behavior is unit-tested headless; rhi gains
  a `gpu_tests::depth_buffer` create/resize test; the existing offscreen GPU
  tests exercise `record_view_pass` through the new signature.

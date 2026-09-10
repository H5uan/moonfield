# Agent Note: Per-view attachment components

Status: implemented

[中文](2026-09-09-per-view-attachment-components.zh.md)

## Problem

The [redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) decided attachments are view-entity components over persistent maps, but the landed code kept a global `ViewTargets` registry keyed by the two-value `RenderTarget` enum, and the pass resolved targets itself. Three consequences: every viewport camera rendered into (and cleared) one shared offscreen target; `PrimaryWindow` views recorded into every in-progress window surface, so two cameras on one target double-cleared the same swapchain image and the last registration order won; and `clear_orphan_view_targets` existed to dim targets no view claimed — a special case the ownership model was supposed to make impossible. Separately, `Core3dPipeline` baked the offscreen color format, so a swapchain with any other format (e.g. sRGB) silently skipped the whole window scene.

## Decision

- `render-core` gains `prepare_view_attachments` (`PrepareViews`): an exclusive system that resolves every `ExtractedView`'s logical target into a per-view `ViewAttachments` component — the color/depth `RenderAttachment` records (view, layout, load/store, clear value), extent, and color format. Viewport views draw into their camera's own pooled target, always cleared. Window views share the window's swapchain image and depth buffer, so the first camera in camera order clears and the rest load — the composite semantics for multiple cameras on one target — with depth stored only when it must survive (more than one camera). Views whose target does not resolve (no acquired image, no pooled target) get no component, and their pass no-ops.
- `ViewTargets` is keyed by the camera's `MainEntity` — stable across frames, unlike render-world view entities, which rebuild every frame — so two viewport cameras get two targets, and `retain_cameras` retires the targets of cameras with no view. `RenderTargetSizes` moves to render-core with the same keying.
- The pool's `ensure` (render-feature, owns the offscreen format constant) orders `.before(&prepare_view_attachments)`; the pass systems are target-enum-free — `opaque_pass_3d` reads `CurrentView`'s `ViewAttachments` and records through the [RenderContext doors](2026-09-09-render-recording-context.md).
- `Core3dPipelines` is a format-keyed map; `prepare_core_3d_pipeline` builds one variant per format the frame's views resolve to (offscreen format, primary surface's format). `Opaque3d` items carry `pipeline: Format`, stamped at queue time from the view's target; `DrawMesh` resolves the variant against the map — a pipeline per draw would be redundant, so the bind deduplicates through the tracked pass. The sRGB-swapchain skip branch is gone: an sRGB target gets its own pipeline variant and the scene draws (hardware applies the encoding).
- `clear_orphan_view_targets` and `record_clear_pass` are deleted; unclaimed pool slots are retired by `retain_cameras` and an editor panel with no viewport view shows its last frame.
- `TextureView` clones share the underlying view and never own it, so attachment records copy views without lifetime plumbing; `RenderAttachment` and `Format` gain the derives the map and records need.

## Alternatives considered

- **Window views broadcast to every in-progress surface (the old loop).** Lost: it recorded views × windows passes per frame and made multi-camera behavior registration-order dependent; until windows carry render-world identity, `PrimaryWindow` resolves to the first in-progress surface by main entity (`WindowSurfaces::primary`).
- **Clear policy as camera data (Bevy's `ClearColorConfig`).** Deferred: the first-clears-rest-load rule needs no `Camera` API change and defines the composite deterministically; a per-camera override lands with a consumer.
- **Item-carried pipeline ids into a shader-keyed cache.** Deferred per the [redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md): the phase has one pipeline per format today; the format stamp is the smallest key that formats actually need. A shader/graphics-state cache arrives with multi-material.

## Consequences

- The three special cases the redesign named for deletion are gone: no orphan-clear system, no shared-viewport-target collision, no window-format skip.
- Two cameras targeting the primary window composite deterministically instead of double-clearing; two viewport cameras render into two targets.
- The editor keys its viewport panel size and texture lookup to the viewport camera's entity, so per-camera targets work end to end.

## Verification

- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (including the GPU readback, depth-occlusion, radix-sort, and sort-pass-order tests) pass; `python3 scripts/verify_agents.py` passes.

# Agent Note: resync architecture.md's renderer section to the set-chain redesign

Status: implemented

[中文](2026-09-13-architecture-doc-render-resync.zh.md)

## Problem

`docs/architecture.md`'s renderer section still described the pre-redesign
render pipeline, contradicting the shipped model recorded in
[render set chain and per-view schedule](../architecture/2026-09-09-render-set-chain-and-per-view-schedule.md)
and
[per-view attachment components](../architecture/2026-09-09-per-view-attachment-components.md).
It spoke of three render-world schedules (`RenderPrepare`/`RenderQueue`/
`Render`) where the code has one `Render` schedule ordered by a set chain; it
described the removed `Core3dFrame`/`Core3dView` types; it claimed the Core3d
pass was format-locked to `VIEW_TARGET_FORMAT` with sRGB swapchains skipped,
where `Core3dPipelines` is a format-keyed map; and it gave offscreen targets a
final `SHADER_READ_ONLY_OPTIMAL` layout, where
[unified image layouts](../architecture/2026-08-26-unified-image-layouts.md)
keeps every non-swapchain image in `GENERAL`.

## Decision

Rewrite the drifted passages in place, keeping the section's structure and
level of detail: the tick and `App::render` descriptions now name
`ExtractSchedule` and the single `Render` schedule with its
`PrepareAssets → Queue → PhaseSort → PrepareViews → CameraDriver → PostViews →
Submit` set chain; the frame-loop systems are described as bracketing that
chain (acquire before it, `create_window_surfaces` after `PrepareAssets`,
submit after `Submit`); the Core3d narrative now runs through per-view
`RenderPhase<Opaque3d>` components, the camera driver's per-view `Core3d`
schedule, `prepare_view_attachments`' `ViewAttachments` resolution, and the
format-keyed `Core3dPipelines`; the layout narrative states the unified
`GENERAL` behavior with `PRESENT_SRC_KHR` for swapchain images; and the
per-draw data description matches the vertex-pulling `DrawData`/`ViewUniforms`
records of
[view uniforms](../architecture/2026-09-04-view-uniforms.md).
Verified sections that still matched the code (extraction, `FrameContext` and
the retire ring, the editor's egui systems) were left untouched apart from
naming the sets the egui systems run in.

## Alternatives considered

- **Rewrite the whole renderer section.** Rejected: most of it still matched
  the code; a full rewrite would churn accurate prose and obscure what the
  redesign actually changed.
- **Point at the Agent Notes instead of restating mechanisms.** Rejected:
  architecture.md is the consolidated mechanism doc; the notes record the
  decisions, the doc must carry the resulting mechanism in prose.

## Consequences

- The renderer section again matches the shipped schedule structure, view
  model, pipeline keying, and layout behavior.
- Two stale doc comments in code (`RenderPrepare` in
  `crates/moonfield-render-feature/src/shader.rs`, `Core3dFrame` in
  `crates/moonfield-render-feature/src/lib.rs`) were found during the audit
  and left for a code-touching change; this pass was docs-only.
- Doc drift has no CI gate; catching it remains a review-time and audit-time
  activity.

# Agent Note: Render pass schedule acceptance

Status: implemented

[中文](2026-09-09-render-pass-schedule-acceptance.zh.md)

## Problem

The [redesign](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) shipped milestone by milestone (schedules as world data, the extract schedule, the set chain with per-view schedules, render commands, the storage image); its last acceptance criterion demanded proof that the machinery serves a feature author: a pass added as a new file plus one registration call, ordered against the opaque pass, recording real GPU work — without touching render-feature core.

## Decision

- `splat/sort_pass.rs` is that pass: `prepare_splat_sort` builds the radix pipelines and pair buffers in `PrepareViews` (the shader source is embedded until the GS integration routes shaders through the asset pipeline); `sort_splats` runs per view in `Core3d`, ordered after `opaque_pass_3d`, recording the sort into the frame command buffer; `register_sort_pass` is the whole registration surface.
- The acceptance test composes the real frame loop: `RenderPlugin` + `RenderFeaturePlugin` + `register_sort_pass` + one camera. A probe system constrained `after(&opaque_pass_3d).before(&sort_splats)` proves the ordering — its constraints are unsatisfiable unless the opaque pass precedes the sort — and the seeded pair set round-trips through the recorded, frame-submitted dispatches to the exact stable CPU sort.
- The placement review passes: the GS forward chain has its home — projection, bucketing, and the sort as per-view systems in `Core3d` after the opaque pass, artifacts in buffers, the blend writing the RGBA16F intermediate, the composite tonemapping into the view target — matching the [GS roadmap](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md) M3 wording.

One gap the acceptance surfaced, deliberately left as-is: `in_set` membership expands at registration time, so appending a set to an existing chain later does not bound earlier members. The splat chain therefore anchors on the opaque pass *system* (`.after(&opaque_pass_3d)`) rather than a splat set; per-set end anchors (Bevy-style whole-set ordering) are the reopening condition when a second feature needs to insert into `Core3d`'s chain.

## Alternatives considered

- **Move the redesign note to `implemented/`.** Lost: implemented notes record shipped reality (Decision/Consequences), not proposals with acceptance checklists; the milestone notes already carry the implementation record, so the redesign note stays proposed with its checkboxes ticked.
- **A splat set in `Core3d`'s chain.** Lost: today it would need the chain registered at once and still order against the opaque pass system for member interleaving; one anchor constraint says the same thing with zero ECS changes.
- **Ordering proof via markers on the opaque pass.** Lost: that edits render-feature core, exactly what the criterion forbids; the constrained probe proves the same edge from outside.

## Consequences

- The redesign's acceptance list is fully ticked; adding a pass is demonstrated as one file plus one call, GPU-verified end to end.
- The sort pass's pairs are synthetic — splat extraction replaces them when GS M3 lands, in the placement this note reviewed.
- The editor regression holds: the editor's 41 tests and the workspace suite pass unchanged, and the viewport/window/egui paths are untouched by the splat feature (it registers nothing without the call).

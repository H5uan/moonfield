# Agent Note: resync architecture.md to the shipped tick model

Status: implemented

[中文](2026-09-06-architecture-doc-resync.zh.md)

## Problem

`docs/architecture.md` had drifted from the code. Its Frame loop and Time
sections still described the pre-refactor design — clocks advanced by the
winit backend, rendering driven outside the tick — contradicting the shipped
model recorded in
[runner and tick aligned to Bevy](../architecture/2026-08-27-runner-and-tick-aligned-to-bevy.md)
and
[TimeUpdateStrategy](../architecture/2026-08-27-time-update-strategy.md).
Smaller drift had accumulated too: the splat fields are `sh_dc`/`sh_rest`,
not `f_dc`/`f_rest`; the dock tabs are titled Outliner/Details/Content
Browser, not Hierarchy/Inspector, and asset loading and scene Save/Load live
in the Content Browser; one `GltfLoader` sentence was a truncated duplicate.
Two mechanisms had no home anywhere: ECS change detection and the
`moonfield-ml` training runtime.

## Decision

Rewrite the Frame loop and Time sections to the shipped model: a tick is
`First` (message swap, clock advance) → the fixed loop → `Update` → the
render pipeline → `Last`; clocks advance via `time_update_system` in `First`
under the `TimeUpdateStrategy` resource, and the backend never touches time.
The same sections now state that the editor binary's plugin stack does not
add `TimePlugin`, so nothing advances its clocks. Add a Change detection
section (per-component ticks, `Ref`/`Mut` wrappers, `MAX_CHANGE_AGE`
clamping, runtime borrow counters, no `Changed`/`Added` filters) and an ML
training section (`moonfield-ml` outside the app framework, Slang autodiff
through the RHI compiler, COLMAP text parsers, `Trainer::run` /
`Adam::record_step` still `todo!()` stubs with `gpu_tests::gaussian_fit` as
the exercised path). Fix the panel names, the splat field names, the broken
`GltfLoader` sentence, the stale `App::update` doc comment, and the
`moonfield-time` roster line in the root `AGENTS.md`.

## Alternatives considered

- **Leave mechanisms to Agent Notes only.** Rejected: notes record decisions;
  architecture.md is the consolidated mechanism doc, and readers need one
  place that describes the tick, change detection, and ML runtime as they
  exist.
- **Delete the stale sections instead of rewriting them.** Rejected: the
  frame loop and time model are exactly the mechanisms the file's intro
  promises to carry; removing them leaves a hole, not a fix.

## Consequences

- architecture.md again matches the code it describes; the one internal
  contradiction between the doc set and the implemented notes is gone.
- Change detection and ML training have a mechanism home, including their
  current limits (no filters, stub loop) stated as fact.
- Doc drift has no CI gate; catching it remains a review-time and
  audit-time activity.

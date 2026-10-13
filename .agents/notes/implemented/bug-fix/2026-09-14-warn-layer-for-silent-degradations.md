# Agent Note: Warn layer for silent degradations

Status: implemented

[中文](2026-09-14-warn-layer-for-silent-degradations.zh.md)

## Problem

Recoverable framework-internal failures were silent: inserting a
relationship pointing at a dead entity discarded it without a trace,
schedule constraints naming a label with no registered system fell back
to registration order invisibly (a typo or renamed system), a
`MeshRenderer` whose mesh has no source path made the whole entity skip
the scene save, and mesh renderers referencing missing assets vanished
from rendering, UI, and logs alike. The editor's Load/Save status
flattened the verdict into text and re-derived it by sniffing the
message for "failed".

## Decision

- Recoverable framework-internal failures warn through the log layer —
  `tracing` directly in moonfield-ecs (below the framework, per
  [the log layering boundary](../architecture/2026-09-05-log-crate-layering-boundary.md)),
  the moonfield-log re-exports above it. The sites: the discarded
  relationship, schedule resolution's ignored labels (warned once per
  rebuild, not per run), the unsaved `MeshRenderer` (`warn_once` at the
  save hook), and mesh renderers referencing missing assets
  (`warn_once` with a count at the extract root — the draw-side skips
  are downstream of the same cause and stay silent).
- The editor's Load/Save status carries its verdict structurally:
  `theme::Status` (Success/Failure) replaces the plain message string,
  and `status_color`'s text sniffing is deleted. Error text stays text —
  `load_with_server`'s `AssetError` → `String` is display-only now that
  the verdict is structural.

## Alternatives considered

- **Result system parameters (the reference implementation's 0.20 shape:
  `IntoResult` + `BevyError`/`Severity` + `ErrorContext` + a fallback
  error handler).** Zero current consumers — every fallible path
  (asset loading, scene save/load) is service-layer code, no system
  returns `Result`. Deferred until the first fallible system exists (ML
  training steps, asset loading becoming a system, or GPU error
  reporting), then built in the full shape rather than piecemeal.
- **A world-level diagnostics resource collecting warnings.** Duplicates
  the log layer every crate already shares.
- **Warning on load-side field resets (mesh color, camera order).** Those
  are systematic, not exceptional: field fidelity is the scene entry
  granularity problem, not a warn.

## Consequences

- moonfield-ecs gains a direct `tracing` dependency, matching how the
  other below-framework leaves log.
- `warn_once` keeps the per-frame sites (save hook probing, mesh
  extraction) from spamming; the schedule warn fires on constraint
  registration changes only.
- Every silent degradation listed above is now a log line away; nothing
  about the degraded behavior itself changes.

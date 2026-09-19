# Agent Note: Explicit primary window resolution

Status: implemented

[中文](2026-09-13-explicit-primary-window-resolution.zh.md)

## Problem

Two places in the window frame loop bypassed the `PrimaryWindow` marker
component that `moonfield-window` defines and `moonfield-winit` spawns:

- `extract_windows` queried only `(&Window, &RawHandleWrapper)`, discarding
  the marker at the world boundary.
- `WindowSurfaces::primary()` resolved the `PrimaryWindow` logical render
  target by guessing: the in-progress surface with the smallest main entity
  bits. That guess silently depends on an undeclared invariant — the
  windowing backend happens to spawn the primary window first — and any
  backend that spawns windows in another order would render to the wrong
  window with no diagnostic.

## Decision

Window identity is carried explicitly end to end:

- `ExtractedWindow` gains a `primary: bool` field; `extract_windows` fills
  it from `Option<&PrimaryWindow>` in its query, the same pattern
  `extract_cameras` uses for `PrimaryCamera`.
- `WindowSurfaceData` keeps a `primary` flag, refreshed from the extracted
  windows every frame by `create_window_surfaces` (the persistent map entry
  cannot read the per-frame component itself).
- `WindowSurfaces::primary()` resolves through a pure function,
  `resolve_primary`, over the in-progress `(entity, is_primary)`
  candidates, with the boundary cases pinned down:
  - exactly one marked candidate: it wins, regardless of entity order;
  - no marked candidate (a backend that never spawns `PrimaryWindow`):
    fall back to the historical smallest-entity guess, so such a backend
    keeps rendering;
  - several marked candidates (a violation of the exactly-one contract):
    `warn_once!` and resolve to the smallest marked entity for
    determinism.
- `resolve_primary` is unit-tested for all four cases (empty, marked-wins,
  fallback, contract violation) with entities built from raw bits — no GPU
  needed.

## Alternatives considered

- **Re-resolve from the `ExtractedWindow` components at every `primary()`
  call.** `primary()` takes `&self` and has no access to the world;
  threading the components in would change the signature and all three
  call sites for no gain over a flag refreshed once per frame.
- **Return `None` when no candidate is marked.** Stricter, but turns an
  unmarked-backend scenario from "wrong window maybe" into "no window
  output at all"; the fallback keeps pre-existing behavior where the guess
  used to be the only mechanism.
- **Panic on multiple marked windows.** The marker contract is exactly-one,
  but a render loop should degrade, not abort, on a content-side mistake;
  `warn_once!` surfaces it without killing the frame.

## Consequences

- The render target `RenderTarget::PrimaryWindow` now follows the marker,
  not spawn order; the undeclared invariant is gone.
- `ExtractedWindow` is a public struct with a new public field; its only
  constructors are inside render-core (`extract_windows` and the clone in
  `create_window_surfaces`), so no downstream crate needed edits.
- `WindowSurfaces::primary()`'s signature is unchanged; the editor and
  render-feature call sites compile untouched.

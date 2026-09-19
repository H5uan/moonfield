# Agent Note: Remove the modal-loop diagnostic scaffolding

Status: implemented

[中文](2026-09-19-remove-modal-loop-diagnostics.zh.md)

## Problem

The [modal sizing loop investigation](../bug-fix/2026-09-13-windows-modal-sizing-loop-frames.md)
left its measurement scaffolding in the source, each block annotated to be
reverted after measurement: a `mod trace` of per-event atomic counters plus a
per-second reporter thread in `moonfield-winit` (gated by
`MOONFIELD_WINIT_TRACE`), a matching `mod trace` of acquire/present outcome
counters in `moonfield-render-core` (gated by `MOONFIELD_RENDER_TRACE`), and
`sim_modal_resize` in `moonfield-winit` — a SendInput-driven real mouse drag
with GDI pixel capture, gated by `MOONFIELD_WINIT_SIM_MODAL_RESIZE`. The
scaffolding also failed `cargo clippy -- -D warnings`: an unused
`PostMessageW` extern and a constant-chunk `chunks_exact` call inside
`sim_modal_resize`.

## Decision

Delete all three blocks and every call site: the `trace::bump` calls in
`moonfield-winit`'s event handlers (`about_to_wait`, `RedrawRequested`,
`Resized`, `run_frame`, `run_modal_frame`) and in
`WindowSurfaceData::acquire_image`/`present`, the `trace::set_modal` calls in
the sizing subclass proc, the env-var gates, and `sim_modal_resize` with its
Win32/GDI externs. The mechanism they measured — the `sizing` module's timer
driving frames through the modal loop — is shipped functionality and stays.

## Alternatives considered

- **Keep the diagnostics permanently behind the env vars.** Rejected: the
  counters exist only to answer the investigation's questions, and those
  answers are recorded in the bug-fix note; what remains is dead scaffolding
  that costs clippy violations and a never-exiting reporter thread per
  enable.
- **Keep the sim repro behind a cargo feature for future regressions.**
  Rejected: the repro drives the real OS desktop (cursor injection,
  foreground stealing), so it cannot run in CI. The editor's
  `MOONFIELD_EDITOR_SIM_RESIZE` hook still covers the swapchain-recreate
  path, and a real drag remains the manual regression check for the modal
  block, as the bug-fix note records.

## Consequences

- The three env vars no longer exist; nothing documented referenced them, so
  `docs/architecture.md` is unchanged.
- `cargo clippy -p moonfield-winit -p moonfield-render-core --all-targets --
  -D warnings` passes; both crates' tests are unchanged and green.
- Frame flow during a modal drag is observable again only through the
  bug-fix note's recorded measurements or a manual drag.

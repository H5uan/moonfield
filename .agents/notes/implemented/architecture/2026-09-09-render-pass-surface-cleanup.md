# Agent Note: Render pass surface cleanup

Status: implemented

[中文](2026-09-09-render-pass-surface-cleanup.zh.md)

## Problem

Four leftovers from the pass-system review: `ViewQuery` shipped in the [redesign's](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.md) M2 with no consumer (per-view passes are exclusive systems that read `CurrentView` manually, so two parallel per-view access mechanisms coexisted); `FrameDrawArena` — frame-paged GPU scratch, engine-level infrastructure next to `FrameContext` — lived in render-feature with two hardcoded allocators (`alloc_view_uniforms`, `alloc_draw_data`) typed for the mesh pipeline's structs; the `MOONFIELD_DEBUG_SCENE` logging seam sat inside `record_view_pass`'s recording body; and the splat sort pass registered through a free function `register_sort_pass(app)` instead of the plugin model everything else uses.

## Decision

- `ViewQuery` is deleted. `CurrentView` plus the exclusive-system read is the one per-view access mechanism; the parameter shape comes back if a non-exclusive per-view system ever needs it.
- `FrameDrawArena` moves to render-core (`arena` module) with `begin_frame_draw_arena`, registered by `RenderPlugin` in `PrepareViews`. The typed allocators collapse into one generic `alloc<T>()`; `ViewUniforms` and `DrawData` stay render-feature-side — only their layout contracts with the shader do.
- The debug seam becomes `debug_scene_log`, a `Queue`-set system ordered after `queue_opaque_3d`: the scene contents log at queue time, and the pass body records only GPU work. Same env var, same once-per-process semantics, per view.
- `register_sort_pass` becomes `SplatSortPassPlugin`. It stays opt-in (the acceptance test adds it; the GS roadmap's M3 wires the real chain), but the registration surface is now a plugin like every other.

## Alternatives considered

- **Extending `ViewQuery` to tuples to justify it.** Lost: composition without a caller is the surface-area growth this review was removing; the exclusive pass systems cannot take `SystemParam`s anyway.
- **Keeping `alloc_view_uniforms`/`alloc_draw_data` as named wrappers.** Lost: the names encoded the two call sites, not a contract; every future pass would add another method to engine infrastructure. `alloc<T>()` is the same mechanism with the type at the call site.
- **Registering `SplatSortPassPlugin` from `RenderFeaturePlugin` now.** Deferred: the sort's pairs are synthetic until splat extraction lands; wiring pointless GPU dispatches into every editor frame buys nothing.

## Consequences

- One per-view access mechanism, one frame-scratch allocation mechanism, debug logging where the data lives (queue), and plugin-shaped feature registration throughout.
- Render-feature no longer owns engine-level GPU infrastructure; its `render_phase.rs` shrinks to phase items, the draw command, and queueing.

## Verification

- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `python3 scripts/verify_agents.py` pass.

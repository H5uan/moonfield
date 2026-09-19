# Agent Note: Documentation corrections from a docs-vs-code audit

Status: implemented

[中文](2026-09-19-doc-claims-vs-code-audit.zh.md)

## Problem

An audit of durable prose against the shipped code found comments and docs
describing mechanisms the code does not implement:

1. `moonfield-ecs`'s change-detection module doc claimed per-system
   `(last_run, this_run)` windows and a periodic tick clamp/rescan at
   `CHECK_TICK_THRESHOLD`. The world holds one global
   `(last_change_tick, change_tick)` window shared by every tick-aware
   accessor, and `increment_change_tick` is a bare `wrapping_add(1)` — no
   clamp, no rescan (`Tick::check_tick` exists but has no caller).
2. `moonfield-time`'s `Time` doc claimed there is no fixed-update schedule
   and the generic clock never swaps to the fixed clock.
   `run_fixed_main_schedule` exists and mirrors `Time<Fixed>` into the generic
   `Time` during each fixed iteration, restoring the virtual clock afterwards.
   The same docs also credited the windowing backend with advancing the
   clocks; `time_update_system` in `First` does that.
3. `moonfield-math`'s `Aabb3d`/`BoundingSphere` docs claimed direct
   uploadability as `vec3`-pair storage-buffer structs, contradicting the same
   crate's `gpu` module: Rust packs two `Vec3`s into 24 bytes while std430
   pads the second `vec3` to offset 16 (32 bytes).
4. `BoundingSphere::merge` was documented as "the tightest" sphere; the
   midpoint-plus-max-radius formula is the tightest sphere *centered at the
   midpoint*, not the minimal enclosing sphere.
5. The core 3D pass claimed "back-face culling" while setting
   `CullMode::None`.

## Decision

Rewrite each passage to describe the shipped mechanism, without implementing
the missing ones (a separate effort if wanted at all). The `Vec3` alignment
rules stay owned by `moonfield-math::gpu`; the volume docs now say the bytes
are `Pod`-uploadable and that the shader-side declaration is the layout's
source of truth. The pass comment states double-sided rasterization neutrally.
`message.rs`'s module doc and the change-detection section of
`docs/architecture.md` were re-verified against the code and needed no change
(no per-system reader windows claimed there; the `MAX_CHANGE_AGE` clamp and
the absence of `Changed<T>`/`Added<T>` filters are real).

## Alternatives considered

- **Implement the missing mechanisms instead (per-system windows, tick
  rescan, back-face culling).** Out of scope: each is a behavioral change
  deserving its own design, and the current world-global window and
  double-sided rasterization are working shipped behavior.
- **Delete the overclaiming sentences without replacement.** Rejected: the
  real mechanism (world-global window, wraparound clamping at
  `MAX_CHANGE_AGE`, the std430 mismatch) is exactly the non-obvious knowledge
  the docs exist to carry.

## Consequences

- Docs and code agree in `change_detection.rs`, `time.rs`, `real.rs`,
  `volumes.rs`, and `core_3d/pass.rs`; `docs/architecture.md` and
  `message.rs` were confirmed accurate as written.
- `CHECK_TICK_THRESHOLD` is documented as only the scale factor for
  `MAX_CHANGE_AGE`; if a tick rescan is ever ported, its doc must be
  re-derived from the new code.

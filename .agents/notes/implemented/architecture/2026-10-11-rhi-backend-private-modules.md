# Agent Note: RHI backend private modules

Status: implemented

**Superseded (platform selection):** the two-arm platform selection is
replaced by the single Vulkan backend — see
[the Vulkan pivot note](2026-10-13-rhi-vulkan-pivot.md). The private-module
layout itself stands.

[中文](2026-10-11-rhi-backend-private-modules.zh.md)

## Problem

The [sub-crate decision](2026-10-09-rhi-backend-subcrates.md) isolated the
backends as separate packages (`moonfield-rhi-core`/`-vulkan`/`-metal`)
behind the facade's re-export lists — the migration starting point the
[layering proposal](../../implemented/architecture/2026-10-10-rhi-interface-and-backend-layering.md)
step 1 prescribed. With the contracts established (the
[sync/memory](2026-10-11-rhi-sync-memory-contract-narrowing.md) and
[graphics recording](2026-10-11-rhi-graphics-recording-contract.md)
contracts, the platform-selected consumers) the proposal's step 3 called for
a reassessment: if the independent backend packages have no concrete
consumer, move them to private modules.

The audit: nothing outside the facade depends on the sub-crates, CI runs
workspace-wide commands (no per-crate references), and the sub-crates'
independent compilation units served no consumer. The in-crate GPU tests —
the one property private modules had to preserve — move along with the
backends as `#[cfg(test)]` modules.

## Decision

`moonfield-rhi` is one crate owning the public interface:

- The former `moonfield-rhi-core` dissolves into the facade root:
  `src/types.rs`, `src/error.rs`, `src/indirect.rs`, `src/shader/` (the
  vocabulary and the shared Slang compiler).
- The backends are private modules: `src/backend/vulkan/` (ash) and
  `src/backend/metal/` (Metal 4), selected at compile time by platform —
  `#[cfg(not(target_vendor = "apple"))]` / `#[cfg(target_vendor = "apple")]`
  in `backend/mod.rs`. The facade's re-export lists carry each platform's
  public items.
- Backend selection is platform-based inside the RHI, per the layering
  proposal: consumers depend on `moonfield-rhi` without fixing a backend.
  The `vulkan`/`metal` cargo features are gone (the mutually-exclusive
  guard with them); the `validation` feature remains as a development
  convenience (a no-op on Apple targets).
- The native stacks are target-gated in `Cargo.toml` (ash, ash-window,
  gpu-allocator on non-Apple; the objc2 stack on Apple), so a selected
  platform's dependency graph excludes the unselected backend's bindings —
  the crate-level dependency restriction the sub-crates carried is now
  platform-level.
- The in-crate GPU tests keep their `pub(crate)` access: they are
  `#[cfg(test)]` modules of the private backends (the smoke-test spelling
  is `cargo test -p moonfield-rhi backend::vulkan::gpu_tests::headless_triangle`).

Supersedes the sub-crate layout decision above; the boundary rule is
unchanged (the facade's re-export lists are the entire public surface,
mechanically enforced by `scripts/verify_rhi_boundary.py`, which now scans
the single `src/` tree).

## Alternatives considered

- **Retain the sub-crates.** Lost: no independent consumer exists, and the
  cross-crate interface bought nothing but a longer members list; the
  in-crate GPU tests survive as private modules.
- **Keep the `vulkan`/`metal` features for dev-side cross-selection.** Lost:
  platform selection is the deployment model (the layering proposal's
  requirement); a feature that selects the other platform's backend invites
  configurations that never ship. The deferred Metal bindless work makes
  the point concrete — the editor does not build on macOS either way.

## Consequences

- Any backend change recompiles the RHI crate — the independent
  compilation-unit property the sub-crates had is given up (the layering
  proposal's risk note anticipated this: a smaller package list does not
  imply faster builds).
- `cfg`-gating replaces crate-level visibility for backend separation;
  `verify_rhi_boundary.py` and the private-module visibility carry the
  discipline the sub-crates' `pub` boundaries enforced structurally.
- The workspace members list is the `crates/*` glob alone; CI needs no
  command changes (it was already workspace-wide).

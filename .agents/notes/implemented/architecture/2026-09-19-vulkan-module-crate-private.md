# Agent Note: The vulkan module tree is crate-private

Status: implemented

[中文](2026-09-19-vulkan-module-crate-private.zh.md)

## Problem

`moonfield-rhi`'s `lib.rs` declared `pub mod vulkan`, so the whole backend
module tree was publicly reachable (`moonfield_rhi::vulkan::device::…`) even
though the [RHI boundary](2026-08-19-vulkan-rhi-boundary.md) intends the crate
root to be the only surface. The boundary rested on every internal member
being individually `pub(crate)`: any `pub` item added inside the tree — and
any re-export forgotten in `vulkan/mod.rs` — leaked a second, uncurated path
to the same types, with `scripts/verify_rhi_boundary.py` as the only net.

## Decision

The declaration is `pub(crate) mod vulkan`; the crate root keeps
`pub use vulkan::*`, which re-exports exactly the curated list in
`vulkan/mod.rs`. The public surface — the names downstream crates import — is
unchanged, so no call site outside the crate changed. Inside the crate
(including the in-crate `gpu_tests`) nothing changes either: `crate::vulkan::…`
paths resolve the same as before.

The import audit that made this cheap: every downstream reference goes through
`moonfield_rhi::<Name>` or `moonfield_rhi::types::…`; no crate in the
workspace names `moonfield_rhi::vulkan` at all.

## Alternatives considered

- **Leave `pub mod vulkan` and police members by review.** Rejected: the
  accidental-public leak is exactly the failure mode the boundary exists to
  prevent, and review-only enforcement already produced the drift this note
  closes.
- **Move the curated re-exports out of `vulkan/mod.rs` into `lib.rs`.**
  Rejected: the list documents the backend's own view of its surface next to
  the modules it selects from; duplicating it at the root adds a second home
  for one fact.
- **Flatten the tree (no `vulkan` module).** Rejected: the module map inside
  `src/vulkan/` (device, swapchain, sync, …) is the crate's working
  organization; renaming every file for a visibility change is churn without
  payoff.

## Consequences

- `moonfield_rhi::vulkan::…` no longer resolves outside the crate; the root
  re-export list in `vulkan/mod.rs` is the single definition of the public
  backend surface, and `verify_rhi_boundary.py` keeps policing what those
  names expose.
- Types that are `pub` inside the tree but absent from the curated list are
  now unreachable downstream; promoting one is an explicit one-line re-export.
- Doc references and SAFETY comments that name `vulkan` module paths describe
  crate-internal layout and remain accurate.

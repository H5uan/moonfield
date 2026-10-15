# Agent Note: Strict GPU contract mode

Status: implemented

[中文](2026-10-14-strict-gpu-contract-mode.zh.md)

## Problem

Unsupported hardware is expected on hosted CI, but a GPU test that returns
without exercising its contract is counted as passing. A strict helper used
only inside the Vulkan backend leaves public RHI contracts, render features,
editor rendering, and ML tests free to skip on a qualified runner.

## Decision

**One public test policy covers every suite.**
`moonfield_rhi::testing::skip_gpu` reports a skip in tolerant mode and
panics with its reason when `MOONFIELD_STRICT_GPU` is set. Backend-local
device and capability probes delegate to it, as do the public RHI contracts,
render-core, mesh/splat, editor, and ML tests. The helper contains no backend
types and has no GPU dependency of its own.

**The strict job runs all feature paths.** The manual `gpu-contract`
workflow targets `self-hosted, gpu` and runs
`cargo test --workspace --all-features --locked` with strict mode set.
Hosted CI retains tolerant behavior. The environment switch is tested through
child processes so tests do not mutate process-global environment state.

## Alternatives considered

**Make hosted CI strict.** Rejected: hosted hardware does not satisfy the
engine's GPU requirements. The strict contract needs a qualified runner.

**Duplicate the skip helper in each crate.** Rejected: independent helpers
can disagree about strict mode and leave newly added suites unguarded.

**Separate strict test binaries or parse skip output.** Rejected: maintaining
a parallel suite drifts from the tested interface, and output parsing is less
direct than failing at the skip site.

## Consequences

- A qualified runner satisfies the Vulkan descriptor heap, device-address
  commands, float32 atomics, and uploader requirements exercised by the suites.
- A failed initialization or capability probe cannot silently pass any GPU
  suite in strict mode.
- The public policy and its environment behavior are testable without GPU
  hardware; ordinary unit tests do not require the strict runner.

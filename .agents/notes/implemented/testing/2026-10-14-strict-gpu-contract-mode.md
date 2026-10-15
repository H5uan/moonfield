# Agent Note: Strict GPU contract mode

Status: implemented

[中文](2026-10-14-strict-gpu-contract-mode.zh.md)

## Problem

The [architecture contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)'s
testing item: the rhi's GPU integration tests skip with an eprintln when no
Vulkan instance or device is available, when the required
`VK_EXT_descriptor_heap` is missing, or when a feature-gated capability
(`VK_KHR_device_address_commands`, `shaderBufferFloat32AtomicAdd`, the
frame uploader) is absent — so CI counts a skip as a pass, and a regression
on hardware that is supposed to run the contract tests is invisible.

## Decision

**`MOONFIELD_STRICT_GPU` turns skips into failures.**
`gpu_tests::common::skip` announces a skip in the tolerant default; when the
env var is set it panics with the skip reason instead, failing the test and
the job. Every skip site in the GPU tests routes through it: instance and
device creation failures, the descriptor-heap probe, the feature-gated
capabilities, and frame-uploader creation.

**The `gpu-contract` workflow is the strict job.** It is manual
(`workflow_dispatch`) and targets a qualified runner (the `self-hosted, gpu`
label); GitHub-hosted runners are not qualified and would rightly fail it.
It runs `cargo test --workspace --locked` with `MOONFIELD_STRICT_GPU` set,
covering the public-contract GPU tests and the representative mesh and
splat paths (`offscreen_triangle`, `gaussian_fit`). Regular CI keeps the
tolerant default.

## Alternatives considered

- **Making the regular `test` job strict.** Lost: `VK_EXT_descriptor_heap`
  is implemented only by recent NVIDIA drivers, so hosted runners would
  fail every push. The strict contract needs its own qualified hardware.
- **A separate strict test binary.** Lost: the same tests already encode the
  contract, and a parallel suite would drift from it. The env var switches
  behavior at the existing skip sites.
- **Counting skips in the workflow and failing on a nonzero total.** Lost:
  parsing test output is brittle; the panic fails at the site with the
  reason, inside the test's own reporting.

## Consequences

- A qualified runner is defined by what the tests demand: a Vulkan device
  with `VK_EXT_descriptor_heap`, device-address commands, buffer float32
  atomics, and a working frame uploader.
- `skip_forced` is split from the env read, so the strict behavior is
  unit-tested without mutating process-global environment state.

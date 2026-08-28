# Agent Note: Optional device extensions with named missing errors

Status: implemented

[中文](2026-08-28-optional-device-extensions.zh.md)

## Problem

`DEVICE_EXTENSIONS` demanded the whole ray-tracing stack unconditionally.
A card without one of them made `vkCreateDevice` fail with a bare
`ERROR_EXTENSION_NOT_PRESENT` — no clue which extension, no way to run with
the feature degraded. This is a real configuration, not a corner case:
Turing-class NVIDIA cards (e.g. T1000) expose no KHR RT extensions at all,
while the software renderer (llvmpipe) does, and mesh rendering plus the
editor's core passes never touch RT.

## Decision

`moonfield-rhi`'s device creation splits extensions into two lists:

- `REQUIRED_DEVICE_EXTENSIONS` — the 8 extensions every supported device must
  expose (swapchain, descriptor heap, extended dynamic state3, mesh shader,
  mutable descriptor type, dynamic vertex input, device generated commands).
  A missing one fails with `Error::DeviceRequest` naming every absent
  extension, replacing the opaque `ERROR_EXTENSION_NOT_PRESENT`.
- `OPTIONAL_DEVICE_EXTENSIONS` — the RT stack as a group (`acceleration
  structure`, `ray tracing pipeline`, `ray query`, `position fetch`, and
  their shared prerequisites `pipeline library` + `deferred host
  operations`) plus `invocation reorder`. Each is enabled only when the
  physical device exposes it, skipped with a `warn!` otherwise.

The corresponding `PhysicalDevice*Features` structures are chained onto
`features2` only when their extension was enabled — the feature request
always matches the enable list. `Device::optional_extension_enabled` lets
consumers query a capability and degrade instead of failing.

## Alternatives considered

**Keep everything required.** Rejected: runnable graphics on an RT-less card
is a requirement the edition is exercised on.

**Fall back to a different physical device (llvmpipe) when the preferred one
lacks extensions.** Rejected: always pick the discrete GPU; a software
fallback hides the missing-extension diagnosis and renders badly anyway.

## Consequences

- T1000-class cards boot the editor with RT disabled (six warnings at
  device creation); mesh, splat, and the UI passes are unaffected.
- Missing required extensions surface by name in the error and the log.
- RT feature code can query `optional_extension_enabled` before creating
  pipelines; nothing in the current consumers gates on it yet.
- `submit_frame_timeline` (the timeline frame loop's device-side submit,
  `feat(render): timeline semaphore frame loop`) lives in the same file;
  see that note for its contract.
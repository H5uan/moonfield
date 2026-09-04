# Agent Note: One buffer vocabulary, cached shaders, cached pipelines

Status: implemented

[中文](2026-09-05-buffer-convergence-and-caches.zh.md)

## Problem

Two buffer vocabularies survived the bindless migration: `Buffer`
(usage-flagged, fixed-function-era) and `GpuAllocation` (BDA carrier with
CPU/GPU views). After egui pulled its geometry, `Buffer`'s only
production user was the offscreen readback. Meanwhile every pipeline
constructor spun up its own Slang compiler and compiled its shaders from
scratch (three compiles per core-3D pipeline build), and pipeline
creation hit the driver cold on every process start.

## Decision

- `GpuAllocation` absorbs `Buffer`: readback goes through
  `GpuAllocation::read_bytes` (a `Memory::Readback` allocation's mapped
  view), the indirect draws take `&GpuAllocation` (the carrier always had
  the `INDIRECT_BUFFER` flag), and `FrameUploader` stages only through
  `upload_alloc`. `Buffer`, `BufferUsage`, and the uploader's
  `upload`/`upload_and_wait` are deleted — one buffer type whose usage
  set covers every way the bindless model touches memory.
- `ShaderCache` becomes the compile path: values are `Arc` (shared
  across threads), a `compile_file_reflection` memoizes reflection
  alongside SPIR-V, and the cache lives on the `Device` as a lazy
  singleton (`device.shader_cache()`), next to the uploader and the heap.
  `ShaderCache` is `Send + Sync` by invariant: every compiler access
  happens under one of its mutexes, and the cached values are plain
  data. Both pipeline constructors compile through it.
- A Vulkan `PipelineCache` rides the same pattern: lazily created,
  seeded from `<XDG_CACHE_HOME or ~/.cache>/moonfield/pipeline_cache.bin`,
  passed to every graphics/compute pipeline create call, and written
  back on `Device::drop`. Rejected (stale/driver-changed) cache data
  only costs a cold start.

## Alternatives considered

- **Keeping `Buffer` for readback and indirect args.** Two vocabularies
  for "a piece of GPU memory" is the exact dual-track the convergence
  evaluation rejected; the absorbed conveniences are one read method and
  a parameter type.
- **A process-global shader cache.** The device owns its singletons;
  a global would outlive device teardown for no gain (SPIR-V is
  device-independent, but pipeline builds go through the device anyway).

## Consequences

- One buffer type: every allocation carries `SHADER_DEVICE_ADDRESS |
  TRANSFER_SRC | TRANSFER_DST | INDIRECT_BUFFER` (plus
  `DESCRIPTOR_HEAP_EXT` for heap backing) — each flag has a real
  consumer.
- Repeated pipeline builds (tests, editor reloads) compile each
  (shader, entry) once per device; driver-side pipeline compilation is
  skipped across runs when the disk cache hits.
- The offscreen readback, `upload_ring`, and `indirect_draw` tests run
  on the allocation path unchanged.

# Agent Note: Upload paths move to the frame uploader

Status: implemented

[中文](2026-08-31-upload-path-to-frame-uploader.zh.md)

## Problem

`Buffer::upload` (GpuOnly) and `Texture::upload` each created a staging
buffer, a command pool, and a one-shot command buffer per call, then blocked
on `queue_wait_idle` — the per-call create/destroy weight the bump arena
exists to remove. The frame uploader had to serve them, but it borrowed
`&Device` and so could not live in ECS resources (`EguiTextures`) or on the
device itself.

## Decision

- **Lifetime-free uploader.** `GpuAllocation::from_resources` builds a block
  from owned `ash::Device` + `Arc<Allocator>`; `GpuBumpAllocator` and
  `FrameUploader` store those owned resources instead of `&Device`, so they
  are `'static`-compatible (also `HostPtr: Sync` under the existing
  single-writer contract). All constructors keep their `&Device` signature.
- **Device-hosted shared uploader.** `Device` lazily builds one
  `OnceLock<Arc<Mutex<FrameUploader>>>` (field declared before `allocator`
  so it drops first, freeing arena chunks while the allocator is alive).
  `Buffer::upload`'s GpuOnly branch routes through
  `uploader.upload_and_wait` — call signatures unchanged, no caller edits.
- **Texture upload delegates.** `Texture::upload` takes `&mut FrameUploader`
  and records through `FrameUploader::upload_image`, which owns the
  layout-transition barriers that used to live in `texture.rs`.
  `begin_frame`/`end_frame` are idempotent: an empty frame submits nothing.
- **Per-frame flush.** `EguiTextures` holds its own `FrameUploader` (the
  `upload_pool` field is gone); `prepare_egui_frame` calls
  `flush_uploads` once at the end of the frame, so all texture deltas go out
  in one submit.
- **Instance extensions are not device extensions.** `VK_KHR_surface` stays
  out of the device enable list: NVIDIA rejects instance extensions there
  with `ERROR_EXTENSION_NOT_PRESENT`, and validation's VUID 01387 noise is
  accepted in exchange for drivers that actually run.

## Alternatives considered

**Explicit uploader threading through every call site.** Rejected: dozens of
callers (mesh loading, tests) would change; the device-hosted shared
uploader keeps `Buffer::upload(device, data)` intact.

**Building a fresh `FrameUploader` per frame in the editor.** Rejected:
per-frame arena allocation (8 MiB) and object churn for no benefit.

## Consequences

- No per-call staging creation anywhere: uploads carve arena memory and go
  out as one submit per frame (textures) or one submit+wait (load-time
  buffers).
- Device-owned uploader drops before the allocator (field order), so chunk
  frees happen while the device and allocator are still valid.
- `upload_ring.rs` exercises the same code the editor runs; the existing
  buffer/tests paths are the regression suite for the shared-uploader route.

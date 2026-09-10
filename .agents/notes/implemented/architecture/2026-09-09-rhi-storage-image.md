# Agent Note: rhi storage image

Status: implemented

[中文](2026-09-09-rhi-storage-image.zh.md)

## Problem

The gaussian-splatting composite needs an intermediate image a compute kernel writes and a raster pass samples (`RGBA16F`, `SAMPLED|STORAGE` usage). The rhi had no storage-capable image constructor, no float color format, and the whole chain was unproven on the descriptor-heap path: `STORAGE_IMAGE` heap descriptors, Slang lowering heap-indexed `RWTexture2D`, and T1000 RGBA16F optimal-tiling storage writes had never run.

## Decision

- `Texture::storage_image(device, uploader, width, height, format)` — the one additive rhi constructor of the redesign's carve-out — creates a `SAMPLED|STORAGE` image with **two descriptor-heap slots over one view**: a storage-image slot for compute `RWTexture2D` writes (`Texture::storage_handle`) and a sampled-image slot for shader reads (`Texture::handle`). `Drop` retires both slots.
- The image's `UNDEFINED -> GENERAL` initialization goes through `FrameUploader::transition_image` (crate-internal) — an image barrier without a copy, the unified-layout guarantee's sanctioned exception; the caller submits with `end_frame` before the first dispatch.
- `Format::R16G16B16A16Sfloat` joins the format enum.
- Device creation requests `shaderStorageImageWriteWithoutFormat` and `shaderStorageImageReadWithoutFormat`: heap-indexed `RWTexture2D` has no declaration site to annotate a format, so Slang emits `OpTypeImage` with `Format=Unknown`.
- The probe (`gpu_tests::storage_image`) proves the full chain: a compute kernel stores `(x, y, 0.5, 1)` per texel through the storage slot, a `barrier(COMPUTE, COMPUTE, Memory)` orders it, and a second dispatch `Texture2D.Load`s the same image through the sampled slot into a readback buffer — exact half-precision values verified on the T1000.

Two silent-failure traps the probe pinned down, now standing knowledge for every heap-shader author:

- **Slang compiles `ResourceDescriptorHeap[…]` accesses to dead code — without error — when the `spvDescriptorHeapEXT` capability is not passed to `compile_*_with_capabilities`.** Both probe kernels need it.
- **The heaps must be bound to the command buffer (`heap.cmd_bind(&cmd)`) before dispatching**, or heap accesses silently read zeros.

## Alternatives considered

- **Zero-fill upload for initialization.** Lost: it forces a `TRANSFER_DST` usage flag and a full-size staging copy just to perform a layout transition; `transition_image` says what it does.
- **A constructor-time format-capability check.** Lost: `Device` does not hold the `Instance`, so `vkGetPhysicalDeviceFormatProperties` is not reachable from the constructor without new plumbing; the probe is the verification, and a fail-fast check can ride along when the GS path integrates for real.
- **Per-type descriptor sizing for the resource heap.** Ruled out by measurement: `vkGetPhysicalDeviceDescriptorSizeEXT` reports `SAMPLED_IMAGE` and `STORAGE_IMAGE` both at 32 bytes on the T1000 (buffers 16, uniforms 8), so the heap's existing image stride covers storage descriptors.

## Consequences

- The GS blend kernel's intermediate is constructible: one call yields both handles, and the composite pass samples through the existing sampled path.
- The two `WithoutFormat` features are requested unconditionally (the T1000 grants them; CI runners without a heap-capable GPU skip device creation anyway).
- `ImageDescriptorKind` (sampled/storage) is crate-internal on `TextureSlotDesc` — the heap's public surface is unchanged, per `verify_rhi_boundary.py`.

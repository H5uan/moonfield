# Agent Note: bindless texture slots

Status: implemented

[中文](2026-09-01-bindless-texture-slots.zh.md)

## Problem

Textures created with `Texture::new` had no way to participate in the
bindless 2.0 descriptor heap: the heap (commit `e3c3363`, `DescriptorHeap`)
owned slots and CPU-visible descriptor memory, but nothing wrote texture
descriptors into it. Bindless shaders index textures by a 32-bit
`TextureHandle`, so textures needed to allocate a slot, upload their pixels,
and write their view's descriptor into the heap — all at creation time.

## Decision

`Texture` gains an optional `slot: Option<TextureSlot>`:

- `TextureSlot` holds `{ handle: TextureHandle, heap: Arc<DescriptorHeap>,
  view_create_info: vk::ImageViewCreateInfo<'static> }`. The create info is
  owned for its *lifetime*: the heap's descriptor write encoded a pointer to
  it (`ImageDescriptorInfoEXT.p_view`), so it must outlive the slot.
- `Texture::bindless(device, uploader, w, h, format, bytes)` is the main
  path: create image + view, queue the upload on the frame uploader, allocate
  a slot, and write the descriptor — one step, returning a texture whose
  `handle()` is the shader-side index. A bytes-length validation guards the
  RGBA8 contract.
- `Texture::new` stays as-is (`slot: None`) for the egui interop escape hatch,
  which still binds through `bind.rs` sets.
- `Drop` returns the slot to the heap first (bump contract: a freed slot is
  never referenced again), then tears down view, image, allocation in the
  existing order.
- The shared heap is built lazily on demand: `Device::descriptor_heap()`
  returns an `Arc<DescriptorHeap>` (OnceLock, like `Device::uploader()`),
  sized by the new `DESCRIPTOR_HEAP_IMAGE_CAPACITY` / `_SAMPLER_CAPACITY`
  constants.

## Alternatives considered

- Keeping the slot allocator outside `Texture` (caller-owned handles): would
  leak slots on drop and split the create+bind atomicity the shader contract
  wants.
- Storing the view handle instead of the create info: the heap encodes a
  create info pointer, not a handle, so the create info must live with the
  slot.

## Consequences

- Bindless textures are self-contained: creation fully prepares a slot, and
  destruction fully returns it.
- Uploads are queued asynchronously on the shared uploader; the caller still
  submits (`end_frame`) before frames that sample the handle.
- egui's `Texture::new` path is untouched and verified by
  `escape_hatch_has_no_slot`.
- Next step: bind the heap into pipelines (`cmd_bind_graphics`) — the
  pipeline-layout integration belongs to the render-phase work.

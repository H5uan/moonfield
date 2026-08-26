# Agent Note: Bindless texture heap (update-after-bind descriptor set)

Status: proposed

[中文](2026-08-21-bindless-texture-heap.zh.md)

## Problem

The bindless compute path (GPU pointer model) is shipped: `GpuAllocation`
pairs a CPU pointer with a device address, compute kernels receive root data
as a single `GpuPtr`, and barriers are stage-only. Missing from the milestone
scope, and required before any graphics pipeline, is the texture model: the
blog's "global indexable texture heap" — a user-visible array of texture
descriptors that shaders index with a 32-bit value, and that the CPU (and
eventually compute) can write directly.

Two candidate Vulkan routes were evaluated against this machine:

- `VK_EXT_descriptor_heap` (2025): the direct analogue of the blog's model.
  Rejected: ash 0.38.0 ships no bindings for it (only `descriptor_buffer`),
  and the local MoltenVK 1.4.323 (and lavapipe) do not expose the extension —
  `vulkaninfo` shows zero `VK_EXT_descriptor_heap` support. A hand-written
  FFI binding cannot help when the driver never enumerates the extension.
- Update-after-bind descriptor sets (`VK_EXT_descriptor_indexing`, Vulkan
  1.2 core feature): the retained-mode-looking route the existing note
  already deferred to as "Partially adopted". This is the route this proposal
  takes, because the machine already supports every required feature bit
  (confirmed via `vulkaninfo`): `descriptorIndexing`, `runtimeDescriptorArray`,
  `descriptorBindingSampledImageUpdateAfterBind`,
  `descriptorBindingVariableDescriptorCount`, `descriptorBindingPartiallyBound`,
  `shaderSampledImageArrayNonUniformIndexing`.

## Proposal

A `texture_heap` module under `moonfield-render/src/vulkan/bindless/`
implements the blog's texture heap as one large update-after-bind descriptor
set, sized at creation and written through `vkUpdateDescriptorSets`. The
public surface mirrors the blog's mental model: a slot is a 32-bit index,
the heap lives for the app lifetime, and shaders sample
`textureHeap[data.textureIndex]`.

### Textures must exist first

Unlike `gpu_alloc` (which allocates memory without a resource object),
textures in this milestone are real Vulkan images: the old Vulkan-based
cover image path (`offscreen.rs`) already has image/image-view/sampler
creation primitives. A minimal `Texture` value holds `(vk::Image,
vk::ImageView, vk::Sampler)` and frees them on drop. The
descriptor-heap part is then purely descriptor bookkeeping on top of
existing images, matching the blog's "texture descriptor creation needs a
thin GPU specific userland API".

## Acceptance criteria

- [ ] `TextureHeap::new(device, capacity)` creates one UAB descriptor pool +
      one descriptor set with a single runtime-array binding of `capacity`
      sampled images; `capacity` is a `u32`.
- [ ] `TextureHeap::alloc_slot()` / `free_slot()` hand out 32-bit indices from
      a free list (bitmap or Vec), starting at 0; a `write(slot, texture)`
      writes an image-view+sampler descriptor via `vkUpdateDescriptorSets`.
- [ ] A kernel samples `texture_heap[index]` through a root struct that
      carries `uint32 textureIndex`; the compute pipeline layout carries one
      empty UAB descriptor set binding plus the existing push-constant range.
- [ ] Headless integration test (lavapipe CI + MoltenVK local) uploads two
      solid-color textures, computes per-slot average via a mini compute
      kernel, reads back, and asserts each slot's value matches its texture.
- [ ] clippy/fmt clean; no new unsafe surface beyond the Vulkan call sites.

## Risks

- MoltenVK has a per-stage update-after-bind ceiling (visible in
  `vulkaninfo`: `maxPerStageDescriptorUpdateAfterBindSampledImages` is
  1,000,000); our capacity fits the machine's actual limit.
- The pipeline layout hard-codes the UAB binding into `ComputePipeline::new`
  (currently push-contant only). Every future compute pipeline creation must
  pass the same heap layout, or descriptor lookup fails in validation. This
  argues for a `ComputePipeline` that takes the heap from the caller, not a
  typo-invisible global.
- The blog's GPU-writeable texture heap (compute writing descriptor data)
  is **not** achieved with UAB descriptor sets — descriptors remain
  CPU-written via `vkUpdateDescriptorSets`. Recorded as the known cost; a
  direct GPU-writeable heap requires `VK_EXT_descriptor_heap` once ash and
  the drivers support it.
- Updating a descriptor slot while the GPU may be reading it is unsafe
  without a `HAZARD_DESCRIPTORS` barrier; the bindless barrier module
  already reserves that hazard flag for exactly this.

## Alternatives considered

- **`VK_EXT_descriptor_heap`**: the ideal, rejected above (no driver, no
  ash bindings on this machine/CI).
- **`VK_EXT_descriptor_buffer` (ash available)**: descriptors as raw GPU
  memory blobs, closest to the blog's "descriptor heap as memory" and
  enabling GPU writes. Rejected for this milestone: MoltenVK's descriptor
  buffer support is not confirmed on this machine, and the probe-based
  validation of the UAB route already covers the graphics path.
- **Small fixed descriptors sets with fast binding swaps (the retained-mode
  pattern)**: rejected — descriptor-set rebinds per draw are exactly the
  cost the blog eliminates; a single big UAB heap removes them for the
  lifetime of the heap.

## Consequences

- Compute and the future graphics pipeline sample any texture by CPU-mapped
  index; material switching is one `uint32` write to the root struct, not a
  descriptor rebind — matching the blog's material-switch use case.
- The RHI now has a second value type alongside `GpuPtr`: `TextureHandle`
  (a `u32` newtype, `Copy`, storable in root structs). It is not a
  retained-mode object.
- `HAZARD_DESCRIPTORS` becomes reachable in the command layer: after
  rewriting a slot that an executing kernel reads, a
  `barrier(Stage, Stage, HAZARD_DESCRIPTORS)`-style call is legal.
- The remaining graphics-pipeline milestone can then be done with zero new
  binding machinery: pixels texture via `texture_heap[data.textureIndex]`.
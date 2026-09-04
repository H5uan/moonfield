# Agent Note: Heap binding at the frame boundary and an immortal sampler cache

Status: implemented

[中文](2026-09-04-frame-boundary-heap-bind-and-sampler-cache.zh.md)

## Problem

Descriptor-heap binding was a per-consumer concern: `record_egui` bound the
heaps before its pass, and nothing guaranteed a frame command buffer had
them bound — the core 3D pass worked only because its shader touches no
heap slot, so the first sampling shader recorded there would have run
against unbound heaps. Sampler slots had the same shape: each consumer
managed its own (the egui pipeline cached by options and freed on drop;
every offscreen target allocated a private slot and retired it on
resize) — one cache duplicated three ways with three lifetime stories.

## Decision

- The frame loop binds both heaps once per frame command buffer, right
  after `begin` (heap binding is command-buffer scoped). Direct
  command-buffer owners — tests — bind their own buffers.
- `DescriptorHeap::sampler_for(desc)` caches one slot per `SamplerDesc`
  and never frees it: distinct descriptions are bounded by configuration
  space, so one slot per description costs less than any reference
  count. `free_sampler_slot` and the `SamplerSlot` retirement action are
  deleted; sampler slots are immortal by design.

## Alternatives considered

- **Reference-counting cached samplers.** Rejected: the machinery
  (counts, release paths, retirement actions) serves a handful of slots
  that configuration space already bounds.
- **Keeping per-consumer caches.** Rejected: three copies of one cache
  and three lifetime stories for the same handful of slots.

## Consequences

- Every frame command buffer has the heaps bound from `begin`; any pass
  recorded in the frame may sample heap slots.
- The egui pipeline's sampler map and its `Drop` are gone
  (`update_texture` takes `&EguiPipeline`); offscreen targets share one
  sampler slot per description, and `HeapSlots` retires only the image
  slot.
- The sampler slot allocator's freelist is never used; the image-slot
  freelist still serves retired image slots.

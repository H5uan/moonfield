# Agent Note: bindless heap sampling (no pipeline descriptor layout)

Status: implemented

[中文](2026-09-01-bindless-heap-sampling.zh.md)

## Problem

The descriptor heap (previous commits) could hold texture descriptors and be
bound to a command buffer, but nothing sampled them: no pipeline read from
the heap on the GPU. The open question was whether the "shader bindings"
channel requires a pipeline descriptor-set layout (one per heap shape) or
whether the heap alone feeds shaders. The Slang capability path
(`ResourceDescriptorHeap` / `spvDescriptorHeapEXT`) looked like the layout-free
route but had never been verified on a real driver — the earlier toolchain
validation ran before this machine's driver had the v2 implementation.

## Decision

Verified empirically and shipped the layout-free route:

- shader.rs: `Compiler` gained capability-aware compilation
  (`compile_*_with_capabilities`); `spvDescriptorHeapEXT` is passed for the
  sampling kernel. The compiler emits `OpCapability UntypedPointersKHR` with
  **no DescriptorSet/Binding decorations** — the true descriptor-heap path,
  so the pipeline needs no set layout and never binds descriptor sets.
- `DescriptorHeap::cmd_bind_graphics` renamed to `cmd_bind`: heap binding is
  command-buffer scoped and bind-point agnostic (one call serves graphics and
  compute).
- device.rs requires `VK_KHR_shader_untyped_pointers` + its feature, and
  upload.rs releases images to `ALL_COMMANDS` (compute samples them too).
- End-to-end test `descriptor_heap_sampling`: a bindless 4x4 red texture →
  `cmd_bind` → compute kernel sampling `ResourceDescriptorHeap[0]` /
  `SamplerDescriptorHeap[0]` → readback asserts solid red, on the real driver.

### Bugs caught while enabling validation

- device.rs chained `PhysicalDeviceFeatures2` with `let _ = features2.push(…)`
  — `push` consumes `self`, so the whole feature chain (bufferDeviceAddress,
  descriptorHeap, timeline, …) was being **discarded**; the device never
  requested those features and drivers silently tolerated it. Now rebound.
- `DeviceCreateInfo` used `push(&mut features2)`; once `features2` heads a
  chain that violates `push`'s chainless-next assertion — merged with
  `extend` instead.
- Note: the local Khronos validation layer (SDK 1.4.335) predates the newer
  structures (reports sType 1000135008 unknown and crashes); counts as a
  toolchain-version gap, not a code defect. All GPU tests pass without it.

## Alternatives considered

- Requiring a descriptor-set layout (the "shader bindings" channel): verified
  unnecessary once the Slang capability path lowers to untyped heap access.
- Keeping `cmd_bind_graphics` for compute use: misleading under the spec's
  bind-point-agnostic binding.

## Consequences

- Bindless 2.0 is complete end to end on real hardware: heap write → bind →
  untyped shader access → sampled readback, with no descriptor set layout,
  no `vkCmdBindDescriptorSets`, and no root signatures beyond a BDA pointer.
- The runtime descriptor heap + BDA pointer pairing now matches the
  no-graphics-API blueprint exactly.
- `VK_KHR_shader_untyped_pointers` joins the required set; CI machines lacking
  it skip the GPU suites as before (they already fail the descriptor_heap
  requirement).

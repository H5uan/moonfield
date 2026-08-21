# Agent Note: VK_EXT_descriptor_heap enabled unconditionally

Status: implemented

[中文](2026-08-21-enable-vk-ext-descriptor-heap.zh.md)

## Problem

The bindless texture heap proposal rejected `VK_EXT_descriptor_heap` for
its milestone because the extension had no ash bindings and no driver
support on the machines it was evaluated against. Both constraints are
gone: the ash git pin ships the generated `ext::descriptor_heap` bindings
(Vulkan-Headers 1.4.352, see the [ash git master
note](2026-08-21-vulkan-1-4-via-ash-git-master.md)), and the platform
target now is recent NVIDIA drivers only. Enabling the extension is the
precondition for the direct GPU-writeable descriptor heap model, replacing
per-draw descriptor-set rebinds with one indexed heap.

## Decision

- `moonfield-render` requests `VK_EXT_descriptor_heap` on every logical
  device in `Device::from_physical_device`: the extension name is listed in
  `DEVICE_EXTENSIONS` next to `VK_KHR_swapchain`, and
  `VkPhysicalDeviceDescriptorHeapFeaturesEXT` with `descriptorHeap` set is
  chained into the `VkPhysicalDeviceFeatures2` pNext chain.
- No fallback: a driver that does not enumerate the extension fails
  `vkCreateDevice`. The RHI targets recent NVIDIA drivers and ships no code
  path that works without the heap.
- GPU integration tests probe the same physical device `Device::new`
  selects; when the extension is absent the test skips with an explicit
  reason (`tests/common/mod.rs`) instead of surfacing the device-creation
  error as a bogus failure. CI on lavapipe stays green.

## Alternatives considered

- **Enable the extension only when enumerated, keep a legacy path**:
  rejected — the RHI targets NVIDIA-only platforms; a silent secondary code
  path is precisely the two-way split the target platform rule excludes.
- **`#[ignore]` the GPU tests and run them on a self-hosted runner**:
  rejected for now — the probe-and-skip helper keeps CI green without
  moving the tests; a self-hosted runner can slot in later with no code
  change.
- **Wait for Mesa to implement the extension**: rejected — lavapipe emits
  no announced support; tests skip there instead.

## Consequences

- Device creation succeeds or fails on driver support for the extension;
  the editor and the GPU tests run only on descriptor-heap-capable drivers.
- `cargo test` stays green on CI: unsupported drivers print the skip
  reason; supported ones run the full suite (the local NVIDIA driver runs
  all GPU tests).
- The texture heap milestone is unblocked: descriptor heaps can back the
  texture model through `VK_BUFFER_USAGE_DESCRIPTOR_HEAP_EXT` buffers,
  `vkWriteResourceDescriptorsEXT` / `vkWriteSamplerDescriptorsEXT`, and
  `vkCmdBindResourceHeapEXT`. The direct-write cost recorded in the
  proposal's risks list no longer applies.
- Descriptor writes move from `vkUpdateDescriptorSets` to direct writes
  into the heap; the `HAZARD_DESCRIPTORS` ordering stays owned by the
  bindless barrier module.
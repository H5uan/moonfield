# Agent Note: Host-visible buffers reuse gpu-allocator's persistent map

Status: implemented

[中文](2026-08-21-host-visible-buffer-reuse-persistent-map.zh.md)

## Problem

Uploading or reading a host-visible (`CpuToGpu` / `GpuToCpu`) buffer failed on
startup with `VK_ERROR_MEMORY_MAP_FAILED: Memory is already mapped`. `Buffer`
allocations go through the device's shared gpu-allocator, which keeps every
host-visible memory block persistently mapped; `mapped_ptr` already points at
the exact allocation region (offset baked in). The old `upload`/`read` paths
called `vkMapMemory` again on the same `VkDeviceMemory`, which MoltenVK rejects.
Lavapipe on CI did not catch this — only strict MoltenVK does. The editor's
`Viewport::new` failed and the viewport never rendered.

## Decision

`moonfield-render::vulkan::buffer::Buffer::upload_host_visible` and `read`
reuse the allocation's `mapped_ptr` when present and only fall back to a manual
map/unmap when the allocation is not persistently mapped. This lands the rule
that host-visible buffers always write through gpu-allocator's persistent
mapping.

## Alternatives considered

- **Call `unmap_memory` before `map_memory`.** Rejected: unmapping a block that
  gpu-allocator keeps mapped breaks the allocator's own mapping contract.
- **Drop `Buffer` and map raw memory per operation.** Rejected: loses the
  allocation/offset and lifetime management gpu-allocator provides.

## Consequences

- Startup no longer fails; the viewport's cube pipeline uploads its vertex and
  index data through the persistent mapping.
- The same rule applies to `read` (viewport dump, test readback), which also
  goes through a host-visible buffer.

# Agent Note: Physical device queries migrated to the v2 API family

Status: implemented

[中文](2026-08-24-physical-device-query-v2.zh.md)

## Problem

The RHI still talked to physical devices through the Vulkan 1.0-era
query entry points while the instance itself is created for Vulkan 1.4
(see the [ash git master note](2026-08-21-vulkan-1-4-via-ash-git-master.md)):
`vkGetPhysicalDeviceProperties(2)` and
`vkGetPhysicalDeviceQueueFamilyProperties(2)` are core since 1.1, and the
KHR `2` surface queries exist for surface capabilities and formats. The
1.0 forms return only their base struct — no pNext chain — so any Vulkan
1.2/1.3/1.4 extended property (e.g. `PhysicalDeviceVulkan13Properties`)
or extended surface capability is unreachable through them, and keeping
two spellings of the same query invites new code to copy the old one.

## Decision

- `Instance::physical_device_properties` is gone; callers use
  `physical_device_properties2(&self, device, out: &mut PhysicalDeviceProperties2)`,
  which takes an output struct so extended structures can be chained
  through its pNext pointer. GPU selection (`Device::new`,
  `RenderPlugin`, `tests/common/mod.rs`) reads `out.properties.device_*`.
- `Instance::queue_family_properties` is gone;
  `queue_family_properties2` returns `Vec<QueueFamilyProperties2>`
  (the ash master binding is a len query + slice fill), and consumers
  read `.queue_family_properties` for the flags.
- `Surface::capabilities` / `formats` now use
  `vkGetPhysicalDeviceSurfaceCapabilities2KHR` /
  `vkGetPhysicalDeviceSurfaceFormats2KHR` and return
  `SurfaceCapabilities2KHR` / `Vec<SurfaceFormat2KHR>` (base data in
  `.surface_capabilities` / `.surface_format`). The shared instance
  enables `VK_KHR_get_surface_capabilities2` on every platform's
  surface-extension list, and `Surface` loads the
  `get_surface_capabilities2` loader next to the surface loader.
- Queries without a `2` variant stay as they were:
  `vkGetPhysicalDeviceSurfacePresentModesKHR` and
  `vkGetPhysicalDeviceSurfaceSupportKHR`.

## Alternatives considered

- **Keep the 1.0 wrappers and add the 2 family alongside**: rejected —
  two parallel spellings for the same query make the "new API" rule
  unenforceable; callers would keep copying the old form.
- **Migrate only the core queries, leave surface queries alone**:
  rejected — the surface "2" queries are the ones that actually benefit
  from pNext (e.g. `SurfaceProtectedCapabilitiesKHR`), and the migration
  cost is one instance extension plus one loader.
- **Keep the v1 convenience return style (`fn() -> T`) and only switch
  the underlying call**: impossible for the 2 family in ash master,
  which returns `()` and fills a caller-provided struct.

## Consequences

- Every physical-device capability/format query in the RHI now goes
  through a "2" entry point, and extended structures can be attached
  without changing the query function again.
- Call sites degrade one level of nesting (`props.device_type` →
  `props.properties.device_type`, `f.format` →
  `f.surface_format.format`); the swapchain selection code binds
  `caps = capabilities.surface_capabilities` once.
- Instances that include surface extensions now also enable
  `VK_KHR_get_surface_capabilities2`; headless instances are unchanged.
- Nothing was gained for identity select — GPU picking and device name
  logging read the same fields as before, only through the 2-shaped
  query.
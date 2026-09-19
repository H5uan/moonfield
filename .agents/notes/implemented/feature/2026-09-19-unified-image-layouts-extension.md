# Agent Note: VK_KHR_unified_image_layouts as an optional device extension

Status: implemented

[中文](2026-09-19-unified-image-layouts-extension.zh.md)

## Problem

The RHI keeps every non-swapchain image in `VK_IMAGE_LAYOUT_GENERAL` (see
[Unified image layouts — GENERAL everywhere](../architecture/2026-08-26-unified-image-layouts.md)).
Without `VK_KHR_unified_image_layouts` that choice is valid everywhere it is
used, but the optimality of `GENERAL` is folklore, not a spec guarantee. The
extension (Vulkan 1.4.313, 2025) blesses exactly this regime: with
`unifiedImageLayouts` enabled, `GENERAL` is legal and spec-guaranteed optimal
for nearly every use. The RHI wants the guarantee on drivers that ship the
extension without breaking the ones that do not.

## Decision

- `ash::khr::unified_image_layouts::NAME` joins `OPTIONAL_DEVICE_EXTENSIONS`
  in `crates/moonfield-rhi/src/vulkan/device.rs`: enabled when enumerated as
  supported, skipped with a warning otherwise — the standing optional pattern
  (see [Optional device extensions](../architecture/2026-08-28-optional-device-extensions.md)).
- The feature bit is probed before enabling. The extension name alone does
  not imply `unifiedImageLayouts`, so device creation chains
  `VkPhysicalDeviceUnifiedImageLayoutsFeaturesKHR` into a
  `vkGetPhysicalDeviceFeatures2` query and drops the candidate when the bit
  is false — the same probe shape as `VK_EXT_shader_atomic_float`. When the
  candidate survives, the same struct with `unifiedImageLayouts(true)` is
  chained into the device-create `pNext`, gated on the enabled list so the
  request matches it exactly. `unifiedImageLayoutsVideo` stays off; the RHI
  does no video coding.
- No local struct or sType definitions were needed: the workspace pins ash to
  git master (`0.38.0+1.4.352`), which already ships
  `ash::khr::unified_image_layouts::NAME` and
  `vk::PhysicalDeviceUnifiedImageLayoutsFeaturesKHR`. The values match the
  Khronos registry (extension 528,
  `VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_UNIFIED_IMAGE_LAYOUTS_FEATURES_KHR =
  1000527000`; Vulkan-Headers `vulkan_core.h`).
- Swapchain layout handling is deliberately unchanged (project-owner
  decision): `AttachmentLayout::Present` stays `PRESENT_SRC_KHR`.
  Presentation is explicitly exempt from the extension, the swapchain already
  performs its one transition per frame, and on the supported desktop targets
  (Windows/Linux) drivers handle `PRESENT_SRC_KHR` at full performance.
- No rendering-code changes: images were already `GENERAL`. Only
  documentation moved — the `AttachmentLayout` doc comment in
  `crates/moonfield-rhi/src/types.rs` and the device.rs extension comments.
- Query exposure needs nothing new: `Device::optional_extension_enabled(&CStr)`
  covers the extension, and no code path branches on the bit, so no dedicated
  accessor was added.

## Alternatives considered

- **Hard-require the extension like `VK_EXT_descriptor_heap`**: rejected —
  the editing machine's AMD driver (Vulkan 1.4.349) does not expose it yet,
  and `GENERAL` remains valid without the extension; an optional enable keeps
  one code path and keeps those drivers working.
- **Move the swapchain to `GENERAL` too**: rejected (owner decision) —
  presentation is exempt from the extension, so nothing would be gained.
- **Hand-define the struct and sType for ash**: unnecessary — that fallback
  existed in case the pinned ash predated Vulkan 1.4.313 headers, but the git
  pin already carries the bindings.
- **A dedicated `unified_image_layouts()` accessor on `Device`**: rejected —
  unlike `buffer_float32_atomic_add`, nothing consumes the bit, and
  `optional_extension_enabled` already exposes it.

## Consequences

- On drivers exposing the extension and the feature bit, `GENERAL` becomes a
  spec-guaranteed-optimal layout for nearly every use; everywhere else the
  behavior is byte-identical to before (still valid, potentially
  non-optimal).
- Device creation costs one extra `vkGetPhysicalDeviceFeatures2` probe on
  drivers that advertise the extension name.
- The unified-image-layouts note claimed opportunistic enabling ahead of the
  code; this change makes that claim true.
- Verified locally: the editing machine's driver does not expose the
  extension, so device creation skips it with the standard warning and all 47
  `moonfield-rhi` tests (including `gpu_tests::headless_triangle`) pass —
  the skip path is exercised end to end; the enable path awaits a driver that
  ships the extension.

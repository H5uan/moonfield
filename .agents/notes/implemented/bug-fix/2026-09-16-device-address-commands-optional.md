# Agent Note: Device-address commands become optional

Status: implemented

[中文](2026-09-16-device-address-commands-optional.zh.md)

## Problem

[Commit 4f1c3ec](../feature/2026-09-12-device-address-commands-and-gpu-timestamps.md)
made `VK_KHR_device_address_commands` unconditionally required, asserting
that every target driver (including the dev machine's T1000) exposes it. The
claim was never checked against the actual hardware: `vulkaninfo` on the dev
machine (Quadro T1000 Mobile, driver 610.43.02, NVIDIA ICD 1.4.341)
enumerates no device exposing the extension - not the T1000, not the Intel
RPL-S, not llvmpipe (`VK_NV_device_address_commands` is absent too). The
extension is a 2024 KHR promotion NVIDIA ships on RTX-20-class cards and
newer; the entry-Turing T1000 sits outside that window. The result is the
startup failure that triggered this note: `RenderPlugin could not initialize
Vulkan: device request failed: physical device is missing required
extensions: ["VK_KHR_device_address_commands"]` on the machine the editor is
developed on.

## Decision

- `VK_KHR_device_address_commands` moves out of `REQUIRED_DEVICE_EXTENSIONS`
  into `OPTIONAL_DEVICE_EXTENSIONS` as a whole capability group (address-based
  indirect draw/dispatch, `cmd_memcpy`, GPU-address timestamp resolves). No
  handle-based fallback replaces it.
- The gate mirrors the existing RT / atomic-float pattern three ways: the
  extension lands in the device enable list only when enumerated; its feature
  struct is pushed onto the pNext chain only when enabled;
  `DeviceExtensionFunctions::device_address_commands` becomes `Option` and
  the loader is built only when the extension was enabled.
- `CommandBuffer`'s five address commands (`draw_indirect`,
  `draw_indirect_count`, `dispatch_indirect`, `cmd_memcpy`,
  `resolve_timestamps`) keep their `GpuPtr`-only signatures and panic with a
  clear message when the loader is `None`. `TimestampQueryPool::new` fails
  with `Error::Unsupported` on devices without the extension, instead of
  building a pool whose results could never be read.
- `Device::device_address_commands()` is the capability query (same shape as
  `buffer_float32_atomic_add`); the four GPU tests that record the commands
  (`bindless_memcpy_dispatch_indirect`, `indirect_draw`, `timestamps`,
  `upload_ring`) skip with an explicit reason when it returns false.
- The `Stage::DRAW_INDIRECT` / `Access::INDIRECT_COMMAND_READ` barrier
  vocabulary stays: a plain stage/access pair with no extension dependency.

## Alternatives considered

- **Handle-based fallback commands.** Mapping `GpuPtr` back to a buffer
  handle needs a reverse address registry or a second command form - both
  violate the address-first single-mechanism invariant, and no production
  caller needs them today.
- **Keep the extension required.** The failure stands: no device on the dev
  machine can create a logical device, blocking the editor and every GPU
  test.
- **Upgrade the driver or swap hardware.** Not a fix: the installed driver
  already exceeds any support threshold, and the extension is genuinely
  absent on entry-Turing hardware.
- **Revert the 2026-09-12 commit.** Restores the handle+offset signatures
  and a host-synced timestamp path, dropping GPU-driven groundwork that has
  no production consumer yet - regresses capable hardware for no gain.

## Consequences

- The editor and the render pipeline initialize again on the dev machine;
  the four command-recording GPU tests skip there, reporting the driver's
  actual capability.
- Callers of the address commands must gate on
  `Device::device_address_commands()`; misuse fails loudly (panic while
  recording, `Unsupported` at pool creation) rather than submitting null
  function pointers.
- The 2026-09-12 note's "all target drivers (including T1000) provide it"
  claim is superseded: entry-Turing does not.
- On drivers that do expose the extension (RTX-20-class and newer) the
  GPU-driven / GPU-address profiling paths are unchanged - optional only
  widens the device floor, it does not regress capable hardware.
# Agent Note: Drop macOS support

Status: implemented

[中文](2026-08-28-drop-macos-support.zh.md)

## Problem

macOS was a windowing target only through MoltenVK, which lags the Vulkan
feature set the RHI targets and forced platform-specific accommodations in the
instance, swapchain, and buffer code, plus a third row in the CI matrices. The
project does not ship to macOS users.

## Decision

The supported targets are Windows and Linux. Concretely:

- CI runs clippy and tests on `ubuntu-latest` and `windows-latest` only, and
  the setup-slang action drops its macOS archive and library-path cases.
- `platform_surface_extensions` in the RHI has no macOS arm; any OS other than
  Windows or Linux requests no surface extensions, so the workspace still
  builds there but windowed rendering fails at surface creation (headless use
  is unaffected).
- `Instance::new` no longer sets `ENUMERATE_PORTABILITY_KHR`; nothing requests
  `VK_KHR_portability_enumeration` anymore.
- The [persistent host-visible mapping](../bug-fix/2026-08-21-host-visible-buffer-reuse-persistent-map.md)
  and [old-swapchain naming](../bug-fix/2026-08-21-swapchain-recreate-names-old-swapchain.md)
  behaviors stay: the Vulkan spec independently requires them (no
  `vkMapMemory` on an already-mapped `VkDeviceMemory`; a replacement swapchain
  names the one bound to the surface). Their comments now cite the spec rule,
  not MoltenVK.
- `NativeKeyCode::MacOS` stays in `moonfield-window`: the enum mirrors winit's
  cross-platform `NativeKeyCode` (which also carries Android and XKB
  variants), so it is not platform support code.

## Alternatives considered

- **Keep macOS as a build-only tier.** Rejected: MoltenVK constrains which
  Vulkan features the RHI can rely on, and an untested target still shapes the
  code for no shipping user.
- **Delete the shared-mapping and old-swapchain accommodations along with
  macOS.** Rejected: both are Vulkan spec requirements, not MoltenVK quirks;
  MoltenVK was merely the driver that enforced them first.
- **Fail compilation outright on macOS (`compile_error!`).** Rejected: the
  workspace must still build on a Mac for non-rendering work (ECS, assets,
  editor logic); a hard error buys nothing over headless-only operation.

## Consequences

- MoltenVK-specific code paths and comments are gone; the RHI targets
  conformant Windows and Linux Vulkan drivers.
- On macOS the app runs under the documented headless tolerance: no
  `RenderDevice` is inserted, windowed consumers retry, and Vulkan tests skip.
- The clippy and test matrices lose one of their three rows.
- MoltenVK and macOS mentions in older notes stay as the historical record of
  why their decisions were made.

# Agent Note: Aggregate device extension loaders

Status: implemented

[English](2026-08-24-aggregated-device-extension-loaders.md)

## Problem

Per-draw dynamic state needs commands Vulkan never promoted to core:
`vkCmdSetColorBlendEnableEXT` / `ColorBlendEquationEXT` / `ColorWriteMaskEXT`
belong to `VK_EXT_extended_dynamic_state3`, so ash exposes them on a separate
`ext::extended_dynamic_state3::Device` loader instead of on `ash::Device`
(which covers core + promoted commands only). The RHI first stored that loader
as a one-off field on `Device`, cloned it into every
`CommandPool`/`CommandBuffer` through a per-extension getter, and wrapped it in
a wgpu-style `ExtensionFn<T>` enum (`Extension`/`Promoted`) — but no extension
this RHI uses has a core counterpart, so `Promoted` was dead code. As more
extensions land, the one-off field + getter pattern becomes a flat pile, with
no single place naming every loaded extension.

## Decision

Follow wgpu-hal's `vulkan/mod.rs` shape, simplified to what this RHI actually
needs:

- `DeviceExtensionFunctions` — one struct aggregating all loaders, held on
  `Device` as `Arc<DeviceExtensionFunctions>` and built once in
  `Device::from_physical_device` (the same shape wgpu's
  `DeviceExtensionFunctions` has inside `Arc<DeviceShared>`).
- `CommandPool`/`CommandBuffer` hold that `Arc` — cloned once at pool
  creation, shared by every command buffer. Loaders are function-pointer
  tables; cloning the `Arc` copies no tables.
- Call-sites access the loader directly as
  `self.ext.extended_dynamic_state3.cmd_set_*` — field access, no
  per-extension getters. `commands` never copies the table; it dereferences
  through the shared `Arc`.

No `ExtensionFn<T>`: the enriched enum added a `Promoted` arm nothing
constructs. When an extension actually gets promoted (none has), the
extension loader field can switch to a `#[allow(dead_code)]`-free marker —
YAGNI until then.

## Alternatives considered

- **`ExtensionFn<T>` with `Extension`/`Promoted`, per-extension getter.**
  Tried first, rejected in review: `Promoted` was never constructed (dead
  code under `-D warnings`), and each call-site needed a
  `self.ext_dynamic_state3()` hop before the real command — noisy compared
  to wgpu, which never writes getters.
- **Keep the single ad-hoc loader field.** Per-extension cost: add a field,
  an accessor, and thread it through `Device` → `CommandPool` → `CommandBuffer`.
  Rejected: multiple extensions would grow into the same flat pile.
- **Stash the whole table on `CommandBuffer` only.** Loaders are
  device-scoped; sharing via `Arc` from the device keeps one source of truth
  and lets other device consumers (e.g. a future GPU-driven recorder) grab
  the same table.

## Consequences

- Adding a new device extension loader is now: one field on
  `DeviceExtensionFunctions`, one load in `from_physical_device`, and one
  `Arc` clone at pool creation — done.
- `CommandPool`/`CommandBuffer` share the table by `Arc`; the hot draw path
  dereferences once through the shared pointer, same indirect-call cost as a
  local field.
- `CullModeFlags` and depth states stay on `ash::Device` core methods — they
  are Vulkan 1.3 core, genuinely promoted, and live separately from the
  extension table on purpose.
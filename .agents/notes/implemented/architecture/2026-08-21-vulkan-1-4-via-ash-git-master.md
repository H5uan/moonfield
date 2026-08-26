# Agent Note: Vulkan 1.4 via ash git master

Status: implemented

[中文](2026-08-21-vulkan-1-4-via-ash-git-master.zh.md)

## Problem

The published ash crate tops out at `0.38.0+1.3.281` (generated from
Vulkan-Headers 1.3.281) and predates the Vulkan 1.4 spec (2025-08). The
bindings lack `API_VERSION_1_4` and the `Vulkan14` structures
(`VkPhysicalDeviceVulkan14Features`/`Properties`), so the RHI cannot
request or inspect a 1.4 instance.

## Decision

- The root `Cargo.toml` patches three vulkan crates from git through
  `[patch.crates-io]`: `ash` and `ash-window` at the ash repository commit
  `f4c2ca3` (`0.38.0+1.4.352`, Vulkan-Headers 1.4.352; ash-window ships in
  the same repository), and `gpu-allocator` at the `ash-next` branch commit
  `6a68a5b` of Traverse-Research/gpu-allocator. Per-crate manifests keep the
  published semver requirements (`ash = "0.38"` and so on); the patched
  versions carry the same `0.38.x` prefix, so every registry requirement in
  the graph, including transitive ones, resolves to the patched source.
- `moonfield-render` call sites migrate to the ash master API, which broke
  in three mechanical ways: extension loaders `Instance::new`/`Device::new`
  become `load`; the pNext builder method `push_next` becomes the
  `TaggedStructure::push` trait method, imported as
  `use ash::vk::{self, TaggedStructure as _}`; `ash_window::create_surface`
  is replaced by `SurfaceFactory::new(...).create_surface(...)`.
- The instance requests the 1.4 API level:
  `ApplicationInfo::api_version(vk::API_VERSION_1_4)`.

## Alternatives considered

- **Stay on the published ash and define the 1.4 additions locally.**
  Rejected: hand-written FFI structs for the 1.4 structures would duplicate
  generated bindings, and the instance version would be a bare numeric
  constant upstream code does not recognize.
- **Wait for the next ash release.** Rejected: ash cuts a release roughly
  once a year, and Vulkan 1.4 support is needed now.
- **Vendor forks of the three crates.** Rejected: vendoring three upstream
  codebases into the tree duplicates their maintenance burden and hides the
  upstream lineage.

## Consequences

- The RHI creates its instance at Vulkan 1.4, and 1.4 core types are in
  scope for device feature/property queries.
- Ash master is not a published release: its source stays compatible with
  the `0.38` semver prefix at the pinned commits, but a future master
  revision can break the API again before the next release; the drift is
  absorbed here when it arrives.
- The `rev` pins keep the build reproducible; `Cargo.lock` records the git
  sources.
- A driver that does not support Vulkan 1.4 fails `create_instance`; driver
  version probing (entry `try_enumerate_instance_version`) is not wired up.
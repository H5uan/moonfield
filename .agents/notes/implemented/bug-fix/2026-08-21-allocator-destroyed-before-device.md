# Agent Note: GPU allocator destroyed before the logical device

Status: implemented

[中文](2026-08-21-allocator-destroyed-before-device.zh.md)

## Problem

Closing the editor window segfaulted on macOS. The crash was in
`MVKDeviceMemory::~MVKDeviceMemory` reached through `vkFreeMemory`, during
`Allocator::drop`. `Device` owns the shared gpu-allocator as
`Arc<Mutex<Allocator>>`; `Device::drop` called `vkDestroyDevice` first, then
the `allocator` field dropped afterward — so its memory blocks were freed
(`vkFreeMemory` / `vkUnmapMemory`) through an already-destroyed logical
device. MoltenVK dereferences freed Objective-C objects during that call and
crashes.

## Decision

`Device::allocator` is now `Option<Arc<Mutex<Allocator>>>`. `Device::drop` takes
the allocator out and, when this is the last `Arc` (every `Buffer`/image
resource drops before its owning device and releases its allocator clone),
destroys it while the device handle is still valid, then calls
`vkDestroyDevice`. The `allocator()` accessor unwraps; it is only `None` during
device drop.

## Alternatives considered

- **Reorder `Device` fields so the allocator drops before `device`.** Rejected:
  the `vkDestroyDevice` call lives in `Drop::drop`'s body, which runs before any
  field drop, so field order cannot help.
- **Change buffers to free before the device.** Rejected: the allocator owns the
  memory blocks, not individual buffers, and the device is the blocking owners
  of the allocator. Releasing the allocator at device teardown is the natural
  single point.

## Consequences

- Closing the editor exits cleanly (status 0) instead of segfaulting.
- The allocator's `try_unwrap` requires every resource's allocator `Arc` to have
  dropped first; if a leaked resource held a clone, `try_unwrap` fails and the
  allocator leaks (safe: no free on a dead device).

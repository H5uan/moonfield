# Agent Note: Bindless RHI GPU pointer model

Status: implemented

[中文](2026-08-19-bindless-rhi-gpu-pointer-model.zh.md)

## Problem

The engine's RHI relies on retained-mode binding objects — `BindGroup`,
`BindGroupLayout`, and a pipeline whose shader inputs are declared through
descriptor sets. Modern GPUs expose bindless access: shaders address data
through raw 64-bit GPU addresses (buffer device address) instead of bound
descriptor sets. A graphics API shaped around that model drops the binding
abstraction layer entirely: shader root data is one GPU pointer per shader
stage, textures are indices into a user-managed heap, and barriers describe
stage-to-stage dependencies without resource lists. This design — proposed by
Sebastian Aaltonen in [No Graphics API][no-gapi] — is the direction this
project wants: it removes the retained-mode objects that currently sit between
scene code and the GPU, and it matches compute-heavy workloads such as
Gaussian splatting.

This note records the plan to build a bindless compute path inside
`moonfield-render` and, eventually, to replace the existing binding model with
it.

[no-gapi]: https://www.sebastianaaltonen.com/blog/no-graphics-api

## Decision

A `bindless` module lives under `moonfield-render/src/vulkan/`, exposed as
`moonfield_render::bindless`. It is a parallel compute-first path; the existing
`BindGroup`/`RenderPass`-based modules stay frozen until the bindless path
covers the graphics pipeline, then the retained modes are deleted in one
switch.

The value types shipped in the first unit: `Memory` with `Default`
(CPU-mapped), `Gpu` (device-local), and `ReadBack`; `GpuPtr(u64)`, the buffer
device address usable in shaders; and `HostPtr`, the CPU view of the same
allocation. The device enables `bufferDeviceAddress` (Vulkan 1.2 core
feature), matching the allocator's BDA support.

### Memory: `gpu_alloc`

`gpu_alloc(device, bytes, align, memory)` returns a `(cpu_ptr, gpu_ptr)` pair:
the CPU pointer is writable directly (UMA or ReBAR-mapped heap), and the GPU
pointer is the buffer device address usable in shaders. `Memory` has
`Default` (CPU-mapped, the common case), `Gpu` (device-local, for textures and
large buffers), and `ReadBack`. The underlying allocator stays `gpu-allocator`;
memory is pooled there and sub-allocated by the bindless layer.

The GPU pointer is a value type `GpuPtr(u64)`, not a handle: it can be stored
in any struct, passed to a shader, and arithmetic-adjusted on the CPU side —
the same design Loon GPU uses. Rust keeps the object-model safety through
ownership (`&`/structs); no `Handle<T>` layer is introduced.

### Probe-cached memory requirements

The engine-side `gpu_alloc` follows the blog's memory-first model: query size
and alignment before allocating, so a resource does not have to exist first.
Vulkan provides no formula for these values — `vkGetBufferMemoryRequirements`
and `vkGetImageMemoryRequirements` only answer for an existing object — so the
engine probes once at device creation: it creates a 1 MiB test buffer (and a
representative image for textures), reads the requirements, destroys the test
objects, and caches the results in the device. Every later allocation applies
the cached alignment and the caller's requested size, without any Vulkan
query. Texture alignment additionally varies with format, dimensions, and
usage; `texture_size_align(desc)` derives those from the probe cache, creating
and destroying a probe object only for first-time combinations.

### Root data

A compute/vertex/fragment shader receives its root data as a single `GpuPtr`
per stage. Local experiment confirmed the Slang toolchain emits
`PhysicalStorageBuffer64` SPIR-V for `Ptr<T, Access.Read>` parameters and
passes the root pointer via the entry point's push constant in stage. The
bindless command layer pushes that pointer as push constant data.

### Queue and synchronization

`queue` — `QueueType::{Graphics, Compute}` — is a first-class value in the
module. The device resolves a dedicated async-compute queue family when one
exists and falls back to the graphics family otherwise; `Device::queue` maps
each `QueueType` to the resolved `vk::Queue`.

Frame pacing uses a timeline semaphore with two frames in
flight. `Semaphore::new_timeline` creates one (initial counter value), and
`Semaphore::wait` blocks the CPU until the counter reaches a value — the
monotonic counter is the same 64-bit object across frames, so ring-buffer
reuse and read-back have a single legal wait point.

`barrier(before, after)` maps to a Vulkan `MemoryBarrier2` with only stage
masks — no resource list. Hazard flags (`HAZARD_DRAW_ARGUMENTS` for GPU-side
draw argument generation) stay a later milestone.

The blog's `gpuSignalAfter`/`gpuWaitBefore` in-memory counters are API
reserved but not implemented in this milestone; the same semantics are
provided today by timeline semaphores.

### Scope of this milestone (compute-only)

- `gpu_alloc` returning CPU/GPU pointer pairs.
- `compute pipeline` and `dispatch` with a `GpuPtr` root pointer.
- `dispatch_indirect` reading launch arguments from GPU memory.
- `cmd_memcpy` for GPU→GPU copies and read-back.
- Pipeline desc is a hashable struct (shader bytes + specialization
  constants) so a `vkCreatePipelineCache`-based cache can be added later
  without changing the public API.

Explicitly out of scope: graphics draws, render passes, texture heaps,
specialization constant caching, and GPU-generated root data per
draw (indirect multi-draw).

## Alternatives considered

- **Author from scratch in a new crate.** Rejected: the retained-mode modules
  stay in `moonfield-render` and the edit surface is already contained to
  `src/vulkan/`; a new crate would split-related Vulkan work across crates.
- **Loon GPU (a C++ implementation of the same proposal) as a reference.**
  Adopted only for its design: `GpuPtr` as a value, no descriptor-set
  bindings in shaders, update-after-bind texture heaps, timeline-frame
  pacing. Not ported wholesale because Metal was rejected and the engine's
  Rust ownership model changes the implementation.
- **`Handle<T>` object handles on the CPU side.** Rejected: Rust's ownership
  model already proves the lifetime statically; a handle table would move the
  same errors to runtime and add lookup + lock. `GpuPtr` and texture
  indices on the GPU's available addressing are values, not handles.
- **Descriptor-set-based bindings with a large update-after-bind heap.**
  Partially adopted: the eventual texture model uses update-after-bind
  descriptor sets. Rejected as the root-data path: shader in-struct pointers
  are the point of this design.

## Consequences

- `PhysicalStorageBuffer` loads/stores are non-coherent by default: kernel
  writes are only visible within the same workgroup. Cross-workgroup
  visibility comes from explicit memory semantics (coherent/volatile
  decorations or atomics); stage-only barriers without the memory model are
  not enough. This is the BDA semantic the blog flags and it shapes the
  barrier design.
- Vulkan provides capture/replay support for buffer device addresses through
  the opaque capture address API, so debugging is not blocked; replay relies
  on the record side keeping the opaque capture address per allocation, which
  `gpu_alloc` carries.
- Slang's `PhysicalStorageBuffer` emission and the push-constant root pointer
  are pinned behavior of the pinned toolchain; upgrading Slang may shift
  layout and needs a compile-time check.
- MoltenVK and lavapipe disagree on some descriptor/feature limits; the
  compute path uses only the shared feature set (documented above).
- A frozen retained-mode path stays in the tree during the transitional
  period, so `cargo clippy` must not warn about the frozen modules as their
  callers move.
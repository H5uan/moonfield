# Agent Note: Bindless RHI GPU pointer model

Status: proposed

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

## Proposal

Add a `bindless` module under `moonfield-render/src/vulkan/`, exposed as
`moonfield_render::bindless`. It is a parallel compute-first path; the existing
`BindGroup`/`RenderPass`-based modules stay frozen until the bindless path
covers the graphics pipeline, then the retained modes are deleted in one switch.

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

### Root data

A compute/vertex/fragment shader receives its root data as a single `GpuPtr`
per stage. Local experiment confirmed the Slang toolchain emits
`PhysicalStorageBuffer64` SPIR-V for `Ptr<T, Access.Read>` parameters and
passes the root pointer via the entry point's push constant in stage. The
bindless command layer pushes that pointer as push constant data.

### Queue and synchronization

`queue` — `QueueType::{Graphics, Compute}` — is a first-class value in the
module, but the initial milestone maps both to the same physical queue; the
abstraction stays so a separate async-compute queue can be introduced without
breaking callers. Frame pacing uses a timeline semaphore with two frames in
flight.

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

## Acceptance criteria

- `gpu_alloc` returns a CPU-writable pointer and a usable`GpuPtr`; CPU writes
  are visible to the GPU.
- A Slang compute shader with a `Ptr<T, Access>` root parameter reads data at
  the pushed GPU address and writes a result buffer.
- `dispatch` launches the kernel; the result is read back to CPU and
  validates the expected value (CPU→GPU→CPU closed loop).
- `barrier(Stage::Compute, Stage::Compute)` runs on the queue without a
  resource list.
- Two frames in flight pace on one timeline semaphore.
- `tests/bindless_compute.rs` passes on both MoltenVK (macOS driver present)
  and lavapipe (CI), reusing the existing Vulkan-presence skip pattern.

## Risks

- Slang's `PhysicalStorageBuffer` emission and the push-constant root pointer
  are pinned behavior of the pinned toolchain; upgrading Slang may shift
  layout and needs a compile-time check.
- MoltenVK and lavapipe disagree on some descriptor/feature limits; the
  compute path uses only the shared feature set (documented above).
- A frozen retained-mode path stays in the tree during the transitional
  period, so `cargo clippy` must not warn about the frozen modules as their
  callers move.
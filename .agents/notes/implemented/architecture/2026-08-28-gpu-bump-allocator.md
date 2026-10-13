# Agent Note: GpuBumpAllocator — grow-on-overflow arena with co-aligned CPU/GPU views

Status: implemented

[中文](2026-08-28-gpu-bump-allocator.zh.md)

## Problem

`GpuAllocation` (bindless) allocates one `VkBuffer`, mapping, and device
address per call — right for persistent resources, wrong for uploads, where
recreating a staging buffer, command pool, and queue wait per upload is the
blocking pattern this phase removes. The upload path needs a cheap
sub-allocator handing out (cpu, gpu) pointer pairs from long-lived
host-visible blocks.

## Decision

`moonfield-rhi`'s `vulkan/bump.rs` owns `GpuBumpAllocator<'a>`, the reference
project's (`no_gfx_api`) `Arena` shape:

- Blocks are `GpuAllocation`s (`Memory::Default`, CpuToGpu); `new` creates the
  first, later ones grow on overflow — never a ring wrap that could land on
  in-flight data.
- Each block records the base alignment it was created with; an allocation
  request exceeding it grows (or rebuilds) a block through
  `GpuAllocation::new_aligned`, which raises `requirements.alignment` before
  allocating — the reference's `mem_requirements.alignment = max(.., align)`.
- `alloc` aligns the offset from the GPU base address, so both views share one
  offset; `check_co_align` errors when the base-pointer delta is not a
  multiple of the requested alignment.
- `free_all` resets to the first block; callers order it after their frame
  signal so every copy sourced from the arena has completed. `block_count`
  reports how many blocks have grown.
- `BumpAlloc` carries the CPU pointer, the device address, and the owning
  buffer + offset for `cmd_copy_buffer`.

`GpuAllocation::new` keeps its signature (delegating to `new_aligned(.., 16)`),
so its 12 existing call sites are untouched.

## Alternatives considered

**The blog's ring wrap (`offset = 0` on overflow).** Rejected: a wrap can
overwrite regions an in-flight frame still reads. The arena grows instead and
leaves reuse to `free_all` after the frame signal.

**Single fixed-size block.** Rejected: overflow would fail instead of growing,
forcing wasteful sizing for per-frame upload bursts.

## Consequences

- Uploads carve from long-lived blocks — no per-call staging buffer, command
  pool, or queue wait; the frame uploader builds on this next.
- Alignment past 16 bytes costs a grown block with a raised base; alignment at
  or below 16 uses the initial block with no extra cost.
- The CPU/GPU co-alignment invariant is checked at block creation; a driver or
  allocator regression surfaces as an error, not a silently misaligned
  pointer.
- `BumpAlloc::src`/`src_offset` are consumed by the frame uploader's copy
  recording (next step).
# Agent Note: command barrier hazard flags and push data

Status: implemented

[中文](2026-09-01-command-barrier-hazard-and-push-data.zh.md)

## Problem

The bindless 2.0 command surface still lacked two pieces of the blog's
vision. The stage barrier — `CommandBuffer::barrier(Stage, Stage)` — always
ordered plain memory read/write hazards: it exposed no way to express that
the hazard involved the descriptor heap, where the CPU writes descriptors
through the host mapping and shaders read them through non-uniform heap
indices. And the extension's own push-constant replacement,
`vkCmdPushDataEXT` (per the extension spec: "update the values of push
data", available to all shaders through the existing PushConstant storage
class, and the fast path for device addresses of shader-constant data), had
no RHI wrapper at all.

## Decision

- `BarrierHazard` enum (in `bindless.rs`, beside `Stage`): `Memory` (the
  previous behavior — MEMORY_READ|MEMORY_WRITE on both sides) and
  `Descriptors`, whose destination access additionally exposes
  `SHADER_SAMPLED_READ` (the sampled-image read a stage performs through a
  heap descriptor). `barrier(before, after, hazard)` replaces the two-arg
  form; the two existing call sites pass `BarrierHazard::Memory`.
- `CommandBuffer::push_data(offset, data)`: wraps
  `vkCmdPushDataEXT` with a `HostAddressRangeConstEXT` over the caller's
  bytes. Like push constants it is offset-addressed, available to all shader
  stages, bounded by `max_push_data_size` at record time (validation flags
  overruns; the RHI does not pre-check — no consumer needs the limit yet).

## Alternatives considered

- Carrying `max_push_data_size` on the RHI types and rejecting oversized
  writes in `push_data`: YAGNI until a push-data pipeline actually consumes
  root data.
- Splitting the hazard into a bitflags set: two kinds exist today, and a
  defaulted enum keeps call sites readable until more arrive.

## Consequences

- Descriptor-heap writes (CPU or a prior GPU stage) can now be ordered
  against sampling explicitly, instead of forcing only wide memory access
  masks.
- `push_data` gives pipelines a larger, offset-addressed root-data channel
  than push constants, ready for the phase-4 pipeline integration.
- Tests: `bindless_barrier` now runs both hazard kinds through the dispatch
  pair (memory + descriptors); `command_push_data` verifies disjoint offsets
  record cleanly.

# Agent Note: A deterministic GPU radix sort in gpu_util

Status: implemented

[中文](2026-09-07-deterministic-radix-sort.zh.md)

## Problem

Splatting orders Gaussians by view-space depth before alpha blending, and the roadmap's standing decision keeps the viewing path bit-exact across runs. An atomic scatter — the simplest sort shape — orders equal-bin items by atomic arrival order, so equal-depth Gaussians would blend differently on every run. The workspace had no GPU sort.

## Decision

`gpu_util::RadixSort` is an LSD radix sort over (u32 key, u32 value) pairs, compiled from the `assets/shaders/util/radix_sort.slang` asset: 8-bit digits, four passes, three dispatches per pass. The histogram (one group per 256-item tile) counts bins in groupshared memory; the scan (one workgroup) exclusively scans the flattened histogram with chunked Hillis-Steele and a running carry; the scatter (one group per tile) places each item at its scanned base plus a wave base plus a lane prefix.

Determinism is structural: every scatter rank derives from scanned offsets and lane order, never from atomic arrival order. The only atomics are counting atomics, whose results are order-independent. The sort is stable — equal keys keep their input order — and reruns reproduce the output bit for bit.

The histogram is bin-major (`hist[bin * groups + group]`): the flat scan orders bins first so every bin's cross-group base is contiguous. The caller passes its own input and output pairs; the sort owns the ping-pong temporaries and the histogram/offsets scratch, reads the input once, and writes the output once. Barriers between the dispatches of one pass are recorded internally; there is no trailing barrier — ordering against other work is the caller's.

## Alternatives considered

- **An atomic scatter (`atomicAdd` on a per-bin counter).** Lost: the within-bin order becomes the atomic arrival order — nondeterministic, so equal keys blend differently across runs and the viewing path loses exactness.
- **Unstable passes with unique composite keys (depth, index).** Lost: unstable LSD radix is incorrect even with unique keys — when the last-processed digit ties, an unstable pass discards the order the earlier digits established.
- **A bitonic network.** Lost: O(n log² n) compare-swaps per view change at splat counts; radix is the reference implementation's shape.
- **A group-major histogram layout (`hist[group * 256 + bin]`).** Lost: the flat scan then orders groups first, every group's bins are contiguous, and each group scatters into its own output segment — groups sort internally and never merge. Bins must be the outer dimension; the acceptance test caught this.

## Consequences

- The acceptance test sorts four cases — 4096 shuffled keys, a partial group, a single element, heavy duplicates — and compares against Rust's stable `sort_by_key` with exact equality, which doubles as the determinism proof.
- Slang facts verified along the way: `groupshared` must be declared at global scope (HLSL allows function scope); `InterlockedAdd` works on groupshared; `WaveGetLaneCount()` needs `spvGroupNonUniform`, which the RHI's `spirv_1_5` session grants through its implicit profile upgrade.
- The scan is a single workgroup; at million-element counts it is the optimization point (a two-level scan), with this API unchanged.
- Splatting maps f32 depth to order-preserving u32 keys at the call site; the sort stays key-agnostic.

# Agent Note: The moonfield-ml training loop runs on the public RHI API

Status: implemented

[中文](2026-09-07-ml-training-loop.zh.md)

## Problem

M1 of the [Gaussian Splatting roadmap](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md) is the training loop: `moonfield-ml` carried its `Trainer`/`TrainingMethod`/`Adam` trait scaffolding with no kernels, no submission loop, and nothing proving the full stack — atomic gradient accumulation, an asset-loaded optimizer kernel, the host loop — through public RHI types only.

## Decision

`Trainer` runs a synchronous loop: one command buffer, re-recorded per step (`begin` → `TrainingMethod::record_step` → `end` → `Device::submit_and_wait`), with the loss read back through `readback_loss` at step 1, every `report_every` steps, and the final step. The command pool is a held-for-drop-order field, the idiom `FrameContext` uses: the command buffer frees itself through the pool handle in `Drop`, so the pool must outlive it.

`Adam` takes a loaded `Shader` asset and compiles from its source through the device `ShaderCache` (`compile_source` / `compile_source_reflection`, both source-text keyed) — training kernels consume the asset layer like any pipeline shader ([2026-09-06-shader-as-asset](../architecture/2026-09-06-shader-as-asset.md)); path selection stays with the app wiring. The kernel's whole side-channel is one `Ptr<AdamConfig>` root parameter: a 24-byte `#[repr(C)]` mirror (step, count, four hyperparameters — six 4-byte scalars, the same natural layout on both sides), with the host advancing `step` through the persistent mapping each dispatch.

The M1 method (`tests/gaussian_fit.rs`) refits the rhi spike's 2D problem through the loop: the backward accumulates each pixel's contribution straight into per-Gaussian gradient slots with `__atomic_add` — the callable spelling for Slang→SPIR-V compiles (the roadmap's standing decision) — and the spike's per-(pixel, Gaussian) records plus reduction pass are gone. The gradient buffer is zeroed host-side at the top of each recorded step; the synchronous loop leaves the GPU idle there, and the production method (M4) clears kernel-side per the roadmap.

## Alternatives considered

- **File-path compilation for the Adam kernel (`ShaderCache::compile_file*` with a baked directory).** Lost: it recreates the `CARGO_MANIFEST_DIR` lookup the shader-as-asset decision deleted, and path-keyed caches cannot see a source change at the same path.
- **Scalar uniform root parameters for the hyperparameters.** Lost: two root-data mechanisms where one suffices; the struct pointer makes every kernel input a pointer and folds the step counter into the same 24 bytes.
- **The spike's record-and-reduce backward.** Lost: the records scale as pixels × Gaussians × parameters — infeasible at scene scale, which is why the roadmap chose atomics.

## Consequences

- `cargo test -p moonfield-ml` proves the loop end to end: the Adam kernel matches a two-step hand computation whose gradients differ per step (0.9, then 0.806770 — differing gradients make a re-zeroed moment buffer and a stuck step counter each observable), and the Gaussian-fit acceptance drives 600 iterations to a final/initial loss ratio ≤ 0.2 through public types only.
- **Slang 2026.14.1 miscompiles the autodiff-plus-atomics combination.** With the lockfile at shader-slang-rs 374b76c the acceptance run finished its iterations and crashed — SIGSEGV under the driver inside `free_command_buffers` at command-buffer teardown. The lockfile resolves e71b713 (Slang 2026.16.1), which compiles and runs the same source correctly; ml needs slang ≥ 2026.16.1.
- Training results are statistical, not bit-reproducible (atomic interleaving); regression criteria are thresholds.
- The step counter is host-written through the persistent mapping each dispatch — safe in the synchronous loop; overlapped submission is roadmap backlog.

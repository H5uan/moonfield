# Agent Note: Gaussian Splatting implementation roadmap

Status: proposed

[中文](2026-09-07-gaussian-splatting-implementation-roadmap.zh.md)

## Problem

The training stack ([2026-09-04-slang-autodiff-gaussian-training](../../implemented/architecture/2026-09-04-slang-autodiff-gaussian-training.md)) and the crate split ([2026-09-04-ml-training-crate](../../implemented/architecture/2026-09-04-ml-training-crate.md)) are decided; the implementation is not sequenced. `moonfield-ml` carries its trait scaffolding with no kernels, `render-feature`'s splat rasterizer and compute utilities are placeholders, and five design points were open: gradient accumulation, sharing the forward kernels between viewing and training, depth sorting, the SH training schedule, and acceptance data.

## Proposal

Standing decisions:

- **Gradient accumulation is buffer float32 atomics** (`VK_EXT_shader_atomic_float`; the dev machine's T1000 exposes `shaderBufferFloat32AtomicAdd`). Slang emits `OpAtomicFAddEXT` through the `__atomic_add` intrinsic — the GLSL-compat `atomicAdd` is not visible to Slang→SPIR-V compiles — verified end to end by `gpu_tests::float_atomics`. Kernels zero the gradient buffers each step. Training results are not bit-reproducible: training-loop regression criteria are statistical (loss/PSNR thresholds), while the viewing path stays exact.
- **One differentiable tile-based forward serves viewing and training.** It writes per-tile sorted Gaussian lists and per-pixel transmittance to buffers; `render-feature` records it as per-view systems in the `Core3d` schedule ([render pass schedule redesign](2026-09-09-render-pass-schedule-redesign.md)), `moonfield-ml` records the same dispatches inside each training step. Viewing pays the artifact-write bandwidth.
- **Depth ordering is the GPU radix sort** in `render-feature::gpu_util`, with the prefix-sum and expansion passes that sorting and tile bucketing need. There is no CPU sort path.
- **Training fits degree-0 SH only**; `sh_rest` stays zero-filled. Higher degrees and their progressive unlock are backlog.
- **Acceptance runs on a public COLMAP scene** (truck from Tandt_db) on the dev machine; CI has no GPU and skips these tests. Thresholds are pinned from the first stable local baseline minus a margin — not from literature peaks — and recorded in this note.

Milestones, each ending in a runnable proof:

- **M1 — the `moonfield-ml` training loop on the public RHI API.** `Trainer::run` over `submit_and_wait`, the Adam kernel (`assets/shaders/ml/adam.slang`), and a minimal `TrainingMethod` refitting the `gaussian_fit` target with an `__atomic_add` backward. The RHI's optional-extension table gains `VK_EXT_shader_atomic_float` with the `buffer_float32_atomic_add` capability.
- **M2 — shared splat math and compute utilities.** `assets/shaders/gs/gaussian.slang` gains 3D covariance, EWA projection, and DC color (all `[Differentiable]`); `gpu_util` gains the radix sort with its scans. The milestone-number comments in `rasterize.rs` and `gpu_util.rs` are removed; this note owns the sequence.
- **M3 — the tile-based forward.** Projection, tile bucketing, radix sort, alpha blending into an RGBA16F image, with the artifacts in buffers; a composite pass tonemaps into the view target ([render pass schedule redesign](2026-09-09-render-pass-schedule-redesign.md)). `render-feature` records it as per-view systems in the `Core3d` schedule (offscreen test first, then the editor viewport); `moonfield-ml` records the same forward per step.
- **M4 — the 3D training loop.** Backward over the tile lists (`[BackwardDerivative]` wrappers for global-memory side effects), per-attribute SoA Adam, L1 loss, `readback_loss`. Dataset views arrive through the COLMAP loader, extended to the binary model, plus an image decoder (a new dependency) with resolution capped to the 4 GB budget.
- **M5 — densification and checkpointing.** Position-gradient accumulation, clone/split/prune with SoA rebuild, checkpoint round-trip, `KHR_gaussian_splatting` export, editor reload.

Backlog, explicitly out of scope: SH degrees 1–3 with the unlock schedule, the D-SSIM loss term, 2DGS / Stoch3DGS kernel families, the ReSTIR integration, an editor training panel, timeline-overlapped submission, camera models beyond PINHOLE/SIMPLE_PINHOLE.

## Alternatives considered

- **Warp/block gradient reduction (deterministic, no extension).** Lost: it complicates the backward kernel to preserve a property only the regression suite consumes, and atomics match the reference implementation's shape. If M1's probe shows Slang cannot emit buffer float atomics on this driver, this alternative reopens.
- **The spike's per-(pixel, Gaussian) gradient buffer plus reduction pass.** Lost: memory grows as pixels × Gaussians × parameters, infeasible at scene scale; it survives only inside the rhi `gaussian_fit` test.
- **A separate non-differentiable viewing rasterizer.** Lost: two kernel families for one algorithm drift apart; the shared forward pays artifact bandwidth on the viewing path instead.
- **CPU depth sort.** Lost: every training step would gain a GPU→CPU sync, putting the CPU back inside the training loop the one-device design removes.
- **All SH degrees trained from step one.** Lost: view-dependent terms carry no signal while positions still move — the reference implementation unlocks degrees progressively for that reason; DC-only shrinks M4 to what its single-view acceptance can measure.
- **Synthetic-scene acceptance only.** Lost: a procedural target passes while COLMAP ingestion, image decoding, and camera math stay untested; the public scene costs a local-only run, and CI runs no GPU tests regardless.

## Acceptance criteria

- M1: `cargo test -p moonfield-ml` refits the `gaussian_fit` target to a final/initial loss ratio ≤ 0.2 through the public API only.
- M2: a GPU probe round-trips permuted keys through the radix sort; covariance and projection match a CPU reference within tolerance.
- M3: an offscreen render test exists, and the editor viewport renders a reference `KHR_gaussian_splatting` glTF.
- M4: `cargo run -p moonfield-ml --example train` overfits one truck view to the pinned loss ratio.
- M5: truck trains to the pinned DC-only PSNR at the capped resolution; export → editor reload round-trips.

## Risks

- Truck's camera model must be PINHOLE/SIMPLE_PINHOLE (the loader's models); a mismatch grows M4.
- The T1000's 4 GB caps image resolution, Gaussian count, and tile-list size; baselines and thresholds are defined at the capped resolution.
- Binary COLMAP parsing and the image decoder are new surface in M4; either can grow beyond estimate.
- Single-view overfitting (M4) can pass despite defects that multi-view training exposes; M5's run is the gate.

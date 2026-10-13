# Agent Note: Gaussian Splatting trains on Slang autodiff over the Lunar Mare RHI

Status: implemented

[中文](2026-09-04-slang-autodiff-gaussian-training.zh.md)

## Problem

The Gaussian Splatting plan (vanilla 3DGS, 2DGS, a Stoch3DGS reproduction, and
a ReSTIR integration) needs a training path: differentiable rendering kernels
plus an optimizer loop. The candidates were the burn deep-learning framework
(wgpu/cubecl backend) and Slang autodiff compiled to SPIR-V running on Lunar
Mare itself. burn would have introduced a second GPU device world — its wgpu
device cannot share Vulkan memory with the RHI's ash device, so every training
step would pay CPU round trips between the two.

## Decision

Training runs on the same Vulkan device as rendering. Differentiable kernels
are written in Slang (`[Differentiable]`, `bwd_diff`, with user-defined
`[BackwardDerivative]` wrappers where global-memory side effects are involved),
compiled to SPIR-V at runtime through the RHI's `Compiler`, and dispatched as
compute. The training runtime a framework would have provided — Adam, loss
kernels, checkpointing — is hand-written; for Gaussian Splatting the parameter
set is the model, so the autodiff graph is one op deep and a framework buys
little.

The spike `crates/moonfield-rhi/tests/gaussian_fit.rs` verifies the path end to
end: 64 2D Gaussians fitted to a 128×128 target by Slang-generated backward
kernels and a hand-written Adam compute kernel, 600 iterations, loss 2662.11 →
35.96 (ratio 0.0135), bit-identical across runs, about 1.4 s. Slang v2026.12
accepts `IDifferentiable` structs, `no_diff` parameters, and `bwd_diff` through
exp/sigmoid/rotation-matrix math without SPIR-V or Vulkan validation errors.

## Alternatives considered

- **burn with the wgpu backend.** Rejected: burn's device is a wgpu device and
  cannot be unified with the RHI's ash device; tensors and RHI buffers would
  exchange through CPU memory every training step. Its core value — deep
  autodiff graphs, kernel autotuning, backend portability — covers needs
  Gaussian Splatting training does not have.
- **burn with the LibTorch/CUDA backend for training only.** Rejected: heavier
  native deployment for a loop whose kernels are hand-written either way (the
  reference 3DGS implementation hand-writes its backward CUDA kernels too), and
  the dual-device split remains.
- **SlangPy or slang-torch.** Rejected: both are host-side bindings for
  Python/PyTorch users; slang-torch is deprecated and CUDA-only, and the
  workspace drives shaders from Rust through Lunar Mare.

## Consequences

- One device world: training parameters, gradients, and render resources share
  one Vulkan device and memory model, so training state renders in place and
  the Stoch3DGS estimator can live as one Slang module shared by the training
  and rendering paths.
- The repo owns its training runtime (Adam, learning-rate scheduling,
  checkpoint serialization, densification bookkeeping); there is no framework
  dependency or upgrade to track.
- Gradient scatter must not rely on floating-point atomics: the spike's
  per-(pixel, gaussian) gradient buffer plus a reduction kernel scales as
  pixels × gaussians and needs warp-level reduction or
  `VK_EXT_shader_atomic_float` at real 3DGS sizes.
- burn is not a workspace dependency. If a later feature needs a real neural
  network beyond what in-kernel Slang autodiff covers, the question is
  revisited then.

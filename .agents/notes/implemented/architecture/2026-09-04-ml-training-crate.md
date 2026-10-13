# Agent Note: moonfield-ml hosts training methods; splat domain types stay in render-feature

Status: implemented

[中文](2026-09-04-ml-training-crate.zh.md)

## Problem

The Slang-autodiff training path (see
[2026-09-04-slang-autodiff-gaussian-training](2026-09-04-slang-autodiff-gaussian-training.md))
needed a home in the workspace. Gaussian Splatting is the first training
method but not the last — burn-based or other ML methods may join later — and
`moonfield-render-feature` already owned the splat domain types
(`GaussianScene`, `SplatCloud`, COLMAP / glTF I/O), with placeholder modules
planning to host training inside the render crate.

## Decision

A new crate `moonfield-ml` is the training runtime: the
`Trainer`/`TrainingMethod` loop scaffolding, the Adam optimizer kernel wiring,
and the `Dataset`/`Checkpoint` traits are method-agnostic; each training
method is one module — `gs` (Gaussian Splatting: 3DGS, 2DGS, Stoch3DGS) is the
first. The crate depends on `moonfield-render-feature` with
`features = ["splat"]` for the domain types; render-feature stays
training-free. Training shaders live under `assets/shaders/ml/`
(method-agnostic kernels such as Adam) and `assets/shaders/gs/` (Gaussian math
shared by training and rendering). The headless training entry is an example
target (`cargo run -p moonfield-ml --example train`); the editor stays the
workspace's only binary.

## Alternatives considered

- **A GS-specific crate (`moonfield-gsplat`).** Rejected: burn-based or other
  ML methods would need a second home later; the runtime scaffolding
  (optimizer, trainer loop, checkpointing) is method-agnostic from the start.
- **Training inside `render-feature::splat` (the original placeholder plan).**
  Rejected: it couples training state into the render-feature crate and blurs
  the split the workspace wants — domain types belong to render-feature,
  training methods belong to the ML crate.
- **Duplicating the splat domain types in `moonfield-ml`.** Rejected:
  `GaussianScene`'s SoA layout contract is already owned by render-feature's
  glTF I/O; two copies of that contract would drift.

## Consequences

- The dependency direction is `moonfield-ml` → `moonfield-render-feature`;
  nothing in the render crates knows training exists, and the editor (which
  can depend on both) may host a training panel later without restructuring.
- 2DGS and Stoch3DGS land as sibling kernel families under `ml::gs`, sharing
  `TrainableScene` and the Adam wiring.
- `GaussianScene`'s SoA layout is the single contract that training, export,
  and rendering all honor.

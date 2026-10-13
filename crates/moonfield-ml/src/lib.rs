//! Moonfield ML: the training runtime on the Lunar Mare RHI.
//!
//! Training runs as compute dispatches on the same Vulkan device the renderer
//! uses: Slang `[Differentiable]` kernels compile to SPIR-V at runtime through
//! the RHI [`Compiler`](moonfield_rhi::Compiler), and the optimizer loop is
//! hand-written — there is no external ML framework dependency (see Agent Note
//! `.agents/notes/implemented/architecture/2026-09-04-slang-autodiff-gaussian-training.md`).
//!
//! Module map:
//!
//! - [`trainer`] — the host-side training loop driving a
//!   [`trainer::TrainingMethod`].
//! - [`optimizer`] — GPU optimizer kernels (Adam).
//! - [`dataset`] — training-view sources.
//! - [`checkpoint`] — parameter checkpoint save/restore.
//! - [`gs`] — the Gaussian Splatting method (3DGS / 2DGS / Stoch3DGS). Domain
//!   types (`GaussianScene`, `SplatCloud`) live in
//!   `moonfield-render-feature::splat`; this module owns only training state.

pub mod checkpoint;
pub mod dataset;
pub mod gs;
pub mod optimizer;
pub mod trainer;

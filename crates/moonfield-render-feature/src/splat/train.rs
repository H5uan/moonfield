//! Per-scene splat training loop.
//!
//! Will own the optimization state for one [`crate::splat::scene::GaussianScene`]:
//! the training loop over COLMAP-registered views, Adam updates on all
//! Gaussian parameters, and adaptive density control (clone/split/prune).
//! Milestones M4–M6.

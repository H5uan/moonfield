//! Backward pass for Gaussian splatting.
//!
//! Will compute gradients of the photometric loss w.r.t. every Gaussian
//! parameter (positions, scales, rotations, opacities, SH coefficients) on
//! the GPU, mirroring the forward rasterizer's tile layout. Milestone M3.

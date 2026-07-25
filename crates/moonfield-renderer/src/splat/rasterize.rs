//! Forward GPU rasterizer for Gaussian splats.
//!
//! Will implement the [`crate::frame::RenderAlgorithm`] phases: extract the
//! visible splats, sort them by view-space depth (radix sort from
//! [`crate::gpu_util`]), and record the tile-based alpha-blending dispatch.
//! Milestone M2 (forward pass), gradient-capable by M3.

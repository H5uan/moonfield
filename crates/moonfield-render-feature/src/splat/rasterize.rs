//! Forward GPU rasterizer for Gaussian splats.
//!
//! Milestone M2 (forward pass), gradient-capable by M3: extract the visible
//! splats, sort them by view-space depth (radix sort from
//! [`crate::gpu_util`]), and record the tile-based alpha-blending dispatch.
//! With the Bevy-aligned architecture these become extraction functions,
//! queue/prepare systems on the render schedule, and a render-graph node —
//! not a per-algorithm trait impl.

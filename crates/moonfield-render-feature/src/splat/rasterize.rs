//! Forward GPU rasterizer for Gaussian splats.
//!
//! Extracts the visible splats, sorts them by view-space depth (the radix
//! sort from [`crate::gpu_util`]), and records the tile-based
//! alpha-blending dispatch. With the Bevy-aligned architecture these become
//! extraction functions, queue/prepare systems on the render schedule, and
//! a render-graph node — not a per-algorithm trait impl.

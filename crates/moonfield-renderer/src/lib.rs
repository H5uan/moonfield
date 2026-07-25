//! Lunaris — scene rendering & algorithms.
//!
//! This crate is the unified container for every high-level rendering
//! algorithm built on top of the Lunar Mare RHI (`moonfield-render`):
//!
//! - `splat` (default feature) — 3D Gaussian splatting: scene representation,
//!   PLY / COLMAP input, GPU rasterization and per-scene training.
//! - `rt` — ray tracing (placeholder).
//! - `gi` — global illumination (placeholder).
//!
//! Frame orchestration follows a simplified version of Bevy's render phases:
//! every algorithm implements [`frame::RenderAlgorithm`] with
//! `extract → prepare → render`. A full render graph is deliberately left
//! out until multiple passes actually need to be composed.

pub mod camera;
pub mod frame;
#[cfg(feature = "gi")]
pub mod gi;
pub mod gpu_util;
#[cfg(feature = "rt")]
pub mod rt;
#[cfg(feature = "splat")]
pub mod splat;

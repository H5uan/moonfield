//! Moonfield render features (Lunaris): scene rendering and algorithms.
//!
//! This crate is the unified container for every high-level rendering
//! algorithm built on the Lunar Mare RHI (`moonfield-rhi`) and the Selene
//! engine layer (`moonfield-render-core`):
//!
//! - `mesh` (default) — the triangle-mesh asset (`Mesh` / `MeshHandle` /
//!   `MeshRenderer`) and its glTF import.
//! - `splat` — 3D Gaussian splatting: scene representation,
//!   `KHR_gaussian_splatting` glTF / COLMAP input, GPU rasterization and
//!   per-scene training. Opt-in; depends on `mesh`.
//!
//! The renderer targets Vulkan through the `moonfield-rhi` RHI.
//!
//! Frame orchestration follows the Bevy-aligned architecture: per-frame
//! extraction produces source-linked views and renderables, asset revisions
//! preserve prepared GPU data, and the render schedule builds a camera-driven
//! [`core_3d::Core3dFrame`] with one sorted opaque phase per view.

pub mod core_3d;
pub mod gpu_util;
#[cfg(feature = "mesh")]
pub mod mesh;
pub mod plugin;
#[cfg(feature = "mesh")]
pub mod render_phase;
#[cfg(feature = "splat")]
pub mod splat;

pub use plugin::RenderFeaturePlugin;

/// The Lunar Mare RHI — single entry point to the rendering backend for all
/// algorithms in this crate (bevy_math-style re-export pattern).
pub use moonfield_rhi as rhi;

#[cfg(test)]
pub(crate) mod test_util {
    /// Serializes tests that create real Vulkan devices or compile shaders:
    /// the Slang compiler (and some drivers) are not thread-safe, so GPU
    /// tests must not run concurrently within one test binary.
    pub(crate) static GPU_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

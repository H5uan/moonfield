//! The single math entry point for the whole moonfield workspace.
//!
//! This crate follows the `bevy_math` pattern: instead of hand-rolling a math
//! library, it re-exports [`glam`] in full and layers a thin set of
//! moonfield-specific domain types and conventions on top. Crates in this
//! workspace should depend on `moonfield-math` only — never on `glam`
//! directly — so the underlying implementation can evolve in one place.
//!
//! The crate serves **both** the graphics renderer and the future GPU-driven
//! physics system. The GPU-facing types are `f32` (Slang compute shaders use
//! `f32`/`f16`); the CPU-side wide-phase and large-world-coordinate math
//! uses the `f64` `D*` variants re-exported from [`glam`].
//!
//! # `no_std` support
//!
//! The crate is `no_std` by default; the `std` feature (enabled by default)
//! re-enables `glam`'s `std`. The `gpu` feature gates nothing extra today — it
//! is a marker for GPU upload support so future `no_std` GPU targets can opt in
//! to `bytemuck` without pulling in the standard library.
//!
//! # Coordinate conventions
//!
//! - **World space** is right-handed, Y-up. A camera with no rotation looks
//!   down -Z, +X is right, +Y is up.
//! - **Clip / NDC space** uses a single engine convention: Y points *up* and
//!   depth is **reverse** (`far -> 0`, near -> 1). Projection matrix
//!   construction (using this convention)
//!   lives in the render crate's camera module, not here. This crate only
//!   provides the low-level [`glam`] primitives (`Mat4::perspective_rh`, etc.).

#![cfg_attr(not(feature = "std"), no_std)]

pub mod bounding;
pub mod direction;
pub mod gpu;
pub mod ray;
pub mod transform;
pub mod volumes;

pub use bounding::{
    BoundingVolume, IntersectsVolume, aabb_from_points, intersects_volume, sphere_from_points,
};
pub use direction::{Dir3, DirError};
pub use ray::Ray3d;
pub use transform::{GlobalTransform, Transform};
pub use volumes::{Aabb3d, BoundingSphere, Frustum, Plane};

pub use glam::*;

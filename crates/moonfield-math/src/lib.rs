//! The single math entry point for the whole moonfield workspace.
//!
//! This crate follows the `bevy_math` pattern: instead of hand-rolling a math
//! library, it re-exports [`glam`] in full and layers a thin set of
//! moonfield-specific domain types and conventions on top. Crates in this
//! workspace should depend on `moonfield-math` only — never on `glam`
//! directly — so the underlying implementation can evolve in one place.
//!
//! # Coordinate conventions
//!
//! - **World space** is right-handed, Y-up. A camera with no rotation looks
//!   down -Z, +X is right, +Y is up.
//! - **Clip / NDC space** uses Vulkan conventions: Y points *down* and depth
//!   maps `near -> 0`, `far -> 1` (the `[0, 1]` range). Projection matrices
//!   must come from the helpers in [`projection`] (e.g.
//!   [`projection::perspective_vk`]); users should never hand-assemble a
//!   projection matrix, because these conventions live in exactly one place.

pub mod direction;
pub mod projection;
pub mod ray;

pub use direction::{Dir3, DirError};
pub use ray::Ray3d;

pub use glam::*;

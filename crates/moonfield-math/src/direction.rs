//! A guaranteed-normalized 3D direction, mirroring `bevy_math`'s `Dir3`.

use std::fmt;
use std::ops::{Deref, Neg};

use crate::Vec3;

/// Error returned when a vector cannot be turned into a [`Dir3`]: it is
/// either zero-length (no direction) or contains non-finite components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirError;

impl fmt::Display for DirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("vector is zero-length or contains non-finite components")
    }
}

impl std::error::Error for DirError {}

/// A 3D direction that is always a unit vector.
///
/// The invariant is established by [`Dir3::new`] (checked) or
/// [`Dir3::new_unchecked`] (caller-guaranteed). `Dir3` derefs to [`Vec3`],
/// so all read-only `Vec3` methods are available directly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dir3(Vec3);

impl Dir3 {
    /// +X.
    pub const X: Self = Self(Vec3::X);
    /// +Y.
    pub const Y: Self = Self(Vec3::Y);
    /// +Z.
    pub const Z: Self = Self(Vec3::Z);
    /// -X.
    pub const NEG_X: Self = Self(Vec3::NEG_X);
    /// -Y.
    pub const NEG_Y: Self = Self(Vec3::NEG_Y);
    /// -Z — the default view direction in moonfield's right-handed world.
    pub const NEG_Z: Self = Self(Vec3::NEG_Z);

    /// Creates a direction from a vector, normalizing it.
    ///
    /// # Errors
    ///
    /// Returns [`DirError`] if `value` is zero-length or contains non-finite
    /// components.
    pub fn new(value: Vec3) -> Result<Self, DirError> {
        if value.is_finite() && value.length_squared() > 0.0 {
            Ok(Self(value.normalize()))
        } else {
            Err(DirError)
        }
    }

    /// Creates a direction without checking the invariant.
    ///
    /// `value` **must** be normalized and finite; feeding in a non-unit or
    /// non-finite vector silently breaks every consumer that relies on the
    /// invariant.
    #[must_use]
    pub const fn new_unchecked(value: Vec3) -> Self {
        Self(value)
    }

    /// The underlying unit vector.
    #[must_use]
    pub const fn as_vec3(&self) -> &Vec3 {
        &self.0
    }
}

impl Deref for Dir3 {
    type Target = Vec3;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Neg for Dir3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl fmt::Display for Dir3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dir3({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_normalizes_non_unit_vector() {
        let dir = Dir3::new(Vec3::new(0.0, 3.0, 4.0)).unwrap();
        assert!(dir.abs_diff_eq(Vec3::new(0.0, 0.6, 0.8), 1e-6));
        assert!((dir.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_new_rejects_zero_vector() {
        assert_eq!(Dir3::new(Vec3::ZERO), Err(DirError));
    }

    #[test]
    fn test_new_rejects_non_finite_vector() {
        assert_eq!(Dir3::new(Vec3::new(f32::NAN, 0.0, 0.0)), Err(DirError));
        assert_eq!(Dir3::new(Vec3::new(f32::INFINITY, 0.0, 0.0)), Err(DirError));
    }

    #[test]
    fn test_neg_flips_direction() {
        assert_eq!(-Dir3::X, Dir3::NEG_X);
        assert_eq!(-Dir3::Z, Dir3::NEG_Z);
        assert_eq!(-(-Dir3::Y), Dir3::Y);
    }

    #[test]
    fn test_axis_constants_are_unit() {
        for dir in [
            Dir3::X,
            Dir3::Y,
            Dir3::Z,
            Dir3::NEG_X,
            Dir3::NEG_Y,
            Dir3::NEG_Z,
        ] {
            assert!((dir.length() - 1.0).abs() < 1e-6);
        }
    }
}

//! A 3D ray, mirroring `bevy_math`'s `Ray3d`.

use crate::{Dir3, Vec3};

/// An infinite half-line with a normalized direction: `origin + t * direction`
/// for `t >= 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray3d {
    /// The point the ray starts from (`t = 0`).
    pub origin: Vec3,
    /// The normalized direction the ray travels in.
    pub direction: Dir3,
}

impl Ray3d {
    /// Creates a ray from an origin and a direction.
    #[must_use]
    pub const fn new(origin: Vec3, direction: Dir3) -> Self {
        Self { origin, direction }
    }

    /// The point at distance `t` along the ray.
    ///
    /// Because the direction is normalized, `t` is a true distance.
    #[must_use]
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + *self.direction * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_at_zero_is_origin() {
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let ray = Ray3d::new(origin, Dir3::X);
        assert_eq!(ray.at(0.0), origin);
    }

    #[test]
    fn test_at_moves_along_direction() {
        let ray = Ray3d::new(Vec3::new(1.0, 0.0, 0.0), Dir3::NEG_Z);
        assert_eq!(ray.at(2.5), Vec3::new(1.0, 0.0, -2.5));
        assert_eq!(ray.at(10.0), Vec3::new(1.0, 0.0, -10.0));
    }
}

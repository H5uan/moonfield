//! Bounding-volume traits shared by the renderer (frustum culling) and the
//! future GPU-driven physics wide phase.
//!
//! These mirror `bevy_math`'s `BoundingVolume` / `IntersectsVolume` traits so
//! that any volume type can be used interchangeably in culling and broad-phase
//! queries. Colliders, constraints, and solvers belong to the future physics
//! crate; the *shapes* and the *intersection tests* live here in math.

use crate::{Aabb3d, BoundingSphere, Vec3};

/// A trait for types that can give a bounding volume approximating themselves.
///
/// Implemented by both [`Aabb3d`] and [`BoundingSphere`] so either can stand in
/// for a culling / broad-phase hull.
pub trait BoundingVolume {
    /// The scalar type used by this volume (usually `f32`).
    type Scalar;
    /// The type used to represent the volume's center (usually [`Vec3`]).
    type Center;
    /// The type used to represent the volume's extents (usually [`Vec3`]).
    type HalfExtents;

    /// The center of the volume.
    fn center(&self) -> Self::Center;

    /// Half the size of the volume along each axis.
    fn half_extents(&self) -> Self::HalfExtents;

    /// The volume as an axis-aligned bounding box.
    fn as_aabb(&self) -> Aabb3d;

    /// Whether this volume intersects `other`.
    fn intersects(&self, other: &Self) -> bool;

    /// Whether this volume fully contains `other`.
    fn contains(&self, other: &Self) -> bool;

    /// A volume that contains both `self` and `other`.
    fn merge(&self, other: &Self) -> Self;

    /// A volume that contains `self`, grown outward by `amount.scalar_length()`.
    fn grow(&self, amount: impl Into<Self::Scalar>) -> Self;
}

/// Whether `self` intersects `volume`.
///
/// Implemented for concrete volume pairs (e.g. ray vs AABB, sphere vs frustum).
pub trait IntersectsVolume<T> {
    /// Returns `true` if `self` intersects `volume`.
    fn intersects(&self, volume: &T) -> bool;
}

/// Helper: `true` if the two volumes intersect, using [`BoundingVolume::intersects`].
#[must_use]
pub fn intersects_volume<A, B>(a: &A, b: &B) -> bool
where
    A: IntersectsVolume<B>,
{
    a.intersects(b)
}

/// A volume that is the axis-aligned box bounding `points`.
///
/// Returns `None` for an empty iterator.
#[must_use]
pub fn aabb_from_points<'a>(points: impl IntoIterator<Item = &'a Vec3>) -> Option<Aabb3d> {
    let mut iter = points.into_iter();
    let mut min = *iter.next()?;
    let mut max = min;
    for p in iter {
        min = min.min(*p);
        max = max.max(*p);
    }
    Some(Aabb3d { min, max })
}

/// A bounding sphere that contains `points`.
#[must_use]
pub fn sphere_from_points(points: &[Vec3]) -> BoundingSphere {
    let center = points.iter().fold(Vec3::ZERO, |acc, p| acc + *p) / points.len() as f32;
    let radius = points
        .iter()
        .map(|p| (*p - center).length())
        .fold(0.0_f32, f32::max);
    BoundingSphere { center, radius }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;

    #[test]
    fn test_aabb_from_points() {
        let pts = [Vec3::new(1.0, 2.0, 3.0), Vec3::new(-1.0, 0.0, 5.0)];
        let aabb = aabb_from_points(pts.iter()).unwrap();
        assert_eq!(aabb.min, Vec3::new(-1.0, 0.0, 3.0));
        assert_eq!(aabb.max, Vec3::new(1.0, 2.0, 5.0));
    }

    #[test]
    fn test_aabb_from_points_empty() {
        let empty: [Vec3; 0] = [];
        assert!(aabb_from_points(empty.iter()).is_none());
    }

    #[test]
    fn test_sphere_from_points_centers() {
        let pts = [Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)];
        let s = sphere_from_points(&pts);
        assert_eq!(s.center, Vec3::new(1.0, 0.0, 0.0));
        assert!((s.radius - 1.0).abs() < 1e-6);
    }
}

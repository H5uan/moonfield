//! Concrete bounding volume types: [`Aabb3d`], [`BoundingSphere`], and
//! [`Frustum`].
//!
//! These serve both the renderer (frustum culling) and the future GPU-driven
//! physics wide phase (broad-phase overlap tests). They are `f32` because they
//! must be uploadable to / comparable with GPU compute shaders; the CPU-side
//! `f64` `D*` variants of [`glam`] are used for large-world accumulation, not
//! for the per-object hulls.

use crate::{bounding::BoundingVolume, Vec3};

/// An axis-aligned bounding box defined by its `min` and `max` corners.
///
/// `#[repr(C)]` + `Pod` so it can be uploaded directly to a GPU storage buffer
/// as a `vec3 min; vec3 max;` pair (see [`crate::gpu`] for the `Vec3` padding
/// caveat).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Aabb3d {
    /// The minimum corner.
    pub min: Vec3,
    /// The maximum corner.
    pub max: Vec3,
}

impl Aabb3d {
    /// An AABB that contains a single point `p` (zero-volume).
    #[must_use]
    pub const fn from_point(p: Vec3) -> Self {
        Self { min: p, max: p }
    }

    /// An AABB that spans `center ± half_extents`.
    #[must_use]
    pub fn from_center_half_extents(center: Vec3, half_extents: Vec3) -> Self {
        Self {
            min: center - half_extents,
            max: center + half_extents,
        }
    }

    /// An AABB that contains `min` and `max` as corners.
    #[must_use]
    pub const fn from_min_max(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// The center of the box.
    #[must_use]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Half the size along each axis.
    #[must_use]
    pub fn half_extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// The full size along each axis.
    #[must_use]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// The box's diagonal length.
    #[must_use]
    pub fn diagonal_length(&self) -> f32 {
        self.size().length()
    }

    /// The box's surface area.
    #[must_use]
    pub fn surface_area(&self) -> f32 {
        let s = self.size();
        2.0 * (s.x * s.y + s.y * s.z + s.z * s.x)
    }

    /// The box's volume.
    #[must_use]
    pub fn volume(&self) -> f32 {
        let s = self.size();
        s.x * s.y * s.z
    }

    /// The box expanded to include `p`.
    #[must_use]
    pub fn union_point(&self, p: Vec3) -> Self {
        Self {
            min: self.min.min(p),
            max: self.max.max(p),
        }
    }

    /// The box expanded to include `other`.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// The box intersected with `other` (empty if they don't overlap).
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        if min.x <= max.x && min.y <= max.y && min.z <= max.z {
            Some(Self { min, max })
        } else {
            None
        }
    }

    /// Whether `p` lies inside the box (inclusive).
    #[must_use]
    pub fn contains_point(&self, p: Vec3) -> bool {
        (self.min.x..=self.max.x).contains(&p.x)
            && (self.min.y..=self.max.y).contains(&p.y)
            && (self.min.z..=self.max.z).contains(&p.z)
    }
}

impl BoundingVolume for Aabb3d {
    type Scalar = f32;
    type Center = Vec3;
    type HalfExtents = Vec3;

    fn center(&self) -> Self::Center {
        self.center()
    }

    fn half_extents(&self) -> Self::HalfExtents {
        self.half_extents()
    }

    fn as_aabb(&self) -> Aabb3d {
        *self
    }

    fn intersects(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    fn contains(&self, other: &Self) -> bool {
        self.min.x <= other.min.x
            && self.max.x >= other.max.x
            && self.min.y <= other.min.y
            && self.max.y >= other.max.y
            && self.min.z <= other.min.z
            && self.max.z >= other.max.z
    }

    fn merge(&self, other: &Self) -> Self {
        self.union(other)
    }

    fn grow(&self, amount: impl Into<Self::Scalar>) -> Self {
        let d = amount.into();
        let half = Vec3::splat(d);
        Self {
            min: self.min - half,
            max: self.max + half,
        }
    }
}

/// A bounding sphere defined by a `center` and a `radius`.
///
/// `#[repr(C)]` + `Pod` so it can be uploaded directly to a GPU storage buffer
/// as a `vec3 center; float radius;` pair (see [`crate::gpu`] for the `Vec3`
/// padding caveat).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BoundingSphere {
    /// The center.
    pub center: Vec3,
    /// The radius.
    pub radius: f32,
}

impl BoundingSphere {
    /// A sphere of radius `r` centered at `center`.
    #[must_use]
    pub const fn new(center: Vec3, radius: f32) -> Self {
        Self { center, radius }
    }

    /// A sphere that contains the two points `a` and `b`, centered at their midpoint.
    #[must_use]
    pub fn from_points(a: Vec3, b: Vec3) -> Self {
        let center = (a + b) * 0.5;
        let radius = a.distance(b) * 0.5;
        Self { center, radius }
    }

    /// The sphere expanded to include `p`.
    #[must_use]
    pub fn union_point(&self, p: Vec3) -> Self {
        let to_p = p - self.center;
        let dist = to_p.length();
        if dist <= self.radius {
            *self
        } else {
            // Grow so the existing sphere and the point both fit.
            let new_radius = (self.radius + dist) * 0.5;
            let center = self.center + to_p * (new_radius - self.radius) / dist;
            Self {
                center,
                radius: new_radius,
            }
        }
    }

    /// Whether `p` lies inside the sphere.
    #[must_use]
    pub fn contains_point(&self, p: Vec3) -> bool {
        self.center.distance(p) <= self.radius
    }
}

impl BoundingVolume for BoundingSphere {
    type Scalar = f32;
    type Center = Vec3;
    type HalfExtents = Vec3;

    fn center(&self) -> Self::Center {
        self.center
    }

    fn half_extents(&self) -> Self::HalfExtents {
        Vec3::splat(self.radius)
    }

    fn as_aabb(&self) -> Aabb3d {
        Aabb3d::from_center_half_extents(self.center, Vec3::splat(self.radius))
    }

    fn intersects(&self, other: &Self) -> bool {
        let d = self.center.distance(other.center);
        d <= self.radius + other.radius
    }

    fn contains(&self, other: &Self) -> bool {
        let d = self.center.distance(other.center);
        d + other.radius <= self.radius
    }

    fn merge(&self, other: &Self) -> Self {
        let center = (self.center + other.center) * 0.5;
        // The tightest sphere centered at the midpoint that contains both.
        let to_other = other.center - center;
        let radius = to_other.length() + other.radius.max(self.radius);
        Self { center, radius }
    }

    fn grow(&self, amount: impl Into<Self::Scalar>) -> Self {
        Self {
            center: self.center,
            radius: self.radius + amount.into(),
        }
    }
}

/// A view frustum, used for frustum culling.
///
/// Encoded as six planes (left, right, bottom, top, near, far) in the outward
/// normal form. The renderer builds it from a view-projection matrix; the
/// planes are extracted from the matrix rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    /// The six planes in order: left, right, bottom, top, near, far.
    pub planes: [Plane; 6],
}

/// A plane in the outward-normal form `normal · p + d <=/>= 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// The outward-facing unit normal.
    pub normal: Vec3,
    /// Distance from the origin along the normal.
    pub d: f32,
}

impl Frustum {
    /// Builds a frustum from a view-projection matrix using the Gribb–Hartmann
    /// plane-extraction method. Plane normals are normalized on extraction.
    #[must_use]
    pub fn from_clip_matrix(m: &crate::Mat4) -> Self {
        // Gribb–Hartmann: rows r0..r3 of the combined view-projection matrix.
        // The six planes are linear combinations of row 3 with each other row.
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let plane = |n: crate::Vec4| -> Plane {
            // n = (normal.xyzw) where the plane is normal·p + d = 0.
            let normal = Vec3::new(n.x, n.y, n.z);
            let d = n.w;
            let len = normal.length();
            Plane {
                normal: normal / len,
                d: d / len,
            }
        };
        Self {
            planes: [
                plane(r3 + r0), // left
                plane(r3 - r0), // right
                plane(r3 + r1), // bottom
                plane(r3 - r1), // top
                plane(r3 + r2), // near
                plane(r3 - r2), // far
            ],
        }
    }

    /// Whether an AABB is at least partially inside the frustum.
    #[must_use]
    pub fn intersects_aabb(&self, aabb: &Aabb3d) -> bool {
        let center = aabb.center();
        let half = aabb.half_extents();
        for plane in &self.planes {
            // Positive vertex: the corner of the AABB furthest along the normal.
            let pv = center + half * plane.normal.signum();
            if plane.normal.dot(pv) + plane.d < 0.0 {
                return false;
            }
        }
        true
    }

    /// Whether a sphere is at least partially inside the frustum.
    #[must_use]
    pub fn intersects_sphere(&self, sphere: &BoundingSphere) -> bool {
        for plane in &self.planes {
            let dist = plane.normal.dot(sphere.center) + plane.d;
            if dist < -sphere.radius {
                return false;
            }
        }
        true
    }
}

// The sphere grows so that both the original sphere and the point fit.
// (The `merge` impl below uses the closed-form midpoint-plus-radius formula.)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aabb_center_and_extents() {
        let a = Aabb3d::from_min_max(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(a.center(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(a.half_extents(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(a.size(), Vec3::new(2.0, 4.0, 6.0));
    }

    #[test]
    fn test_aabb_contains_point() {
        let a = Aabb3d::from_min_max(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        assert!(a.contains_point(Vec3::new(0.5, 0.5, 0.5)));
        assert!(a.contains_point(Vec3::new(0.0, 0.0, 0.0)));
        assert!(!a.contains_point(Vec3::new(1.5, 0.5, 0.5)));
    }

    #[test]
    fn test_aabb_intersects() {
        let a = Aabb3d::from_min_max(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        let b = Aabb3d::from_min_max(Vec3::new(0.5, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0));
        assert!(a.intersects(&b));
        let c = Aabb3d::from_min_max(Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 1.0, 1.0));
        assert!(!a.intersects(&c));
    }

    #[test]
    fn test_aabb_union_and_intersection() {
        let a = Aabb3d::from_min_max(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        let b = Aabb3d::from_min_max(Vec3::new(0.5, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0));
        let u = a.union(&b);
        assert_eq!(u.min, Vec3::ZERO);
        assert_eq!(u.max, Vec3::new(2.0, 1.0, 1.0));
        let i = a.intersection(&b).unwrap();
        assert_eq!(i.min, Vec3::new(0.5, 0.0, 0.0));
        assert_eq!(i.max, Vec3::new(1.0, 1.0, 1.0));
        assert!(a
            .intersection(&Aabb3d::from_min_max(
                Vec3::new(5.0, 5.0, 5.0),
                Vec3::new(6.0, 6.0, 6.0)
            ))
            .is_none());
    }

    #[test]
    fn test_sphere_intersects() {
        let a = BoundingSphere::new(Vec3::ZERO, 1.0);
        let b = BoundingSphere::new(Vec3::new(1.5, 0.0, 0.0), 1.0);
        assert!(a.intersects(&b));
        let c = BoundingSphere::new(Vec3::new(3.0, 0.0, 0.0), 1.0);
        assert!(!a.intersects(&c));
    }

    #[test]
    fn test_sphere_union_point() {
        let s = BoundingSphere::new(Vec3::ZERO, 1.0);
        let grown = s.union_point(Vec3::new(4.0, 0.0, 0.0));
        // Center shifts halfway to the point (1.5); radius grows to 2.5 so
        // both the original sphere and the point fit exactly.
        assert!((grown.center.x - 1.5).abs() < 1e-6);
        assert!((grown.radius - 2.5).abs() < 1e-6);
        // Both the original sphere and the point are contained.
        assert!(grown.contains_point(Vec3::ZERO));
        assert!(grown.contains_point(Vec3::new(4.0, 0.0, 0.0)));
    }

    #[test]
    fn test_frustum_trivial_contains() {
        // A frustum from an identity clip matrix should roughly contain the origin.
        let m = crate::Mat4::IDENTITY;
        let f = Frustum::from_clip_matrix(&m);
        let aabb = Aabb3d::from_point(Vec3::ZERO);
        assert!(f.intersects_aabb(&aabb));
    }
}

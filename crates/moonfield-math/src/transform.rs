//! Local/global transform types used by the ECS hierarchy.
//!
//! These are plain math types with no ECS knowledge: `Transform` is a local
//! TRS (translation/rotation/scale), `GlobalTransform` is the composed world
//! affine. The ECS side (`moonfield-ecs`) attaches them as components and
//! propagates parents to children.

use crate::{Affine3A, Mat3, Mat4, Quat, Vec3};

/// A local transform: translation, rotation, and scale relative to the
/// parent (or to the world for hierarchy roots).
///
/// Composition order mirrors Bevy: scale, then rotate, then translate.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "std", derive(moonfield_reflect::Reflect))]
pub struct Transform {
    /// Position relative to the parent.
    pub translation: Vec3,
    /// Rotation relative to the parent.
    pub rotation: Quat,
    /// Scale relative to the parent.
    pub scale: Vec3,
}

impl Transform {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// A transform with the given translation and identity rotation/scale.
    pub const fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::IDENTITY
        }
    }

    /// A transform with the given rotation and identity translation/scale.
    pub const fn from_rotation(rotation: Quat) -> Self {
        Self {
            rotation,
            ..Self::IDENTITY
        }
    }

    /// A transform with the given scale and identity translation/rotation.
    pub const fn from_scale(scale: Vec3) -> Self {
        Self {
            scale,
            ..Self::IDENTITY
        }
    }

    /// A transform with the given translation and identity rotation/scale.
    pub fn from_xyz(x: f32, y: f32, z: f32) -> Self {
        Self::from_translation(Vec3::new(x, y, z))
    }

    /// The affine matrix of this transform: `T * R * S`.
    #[inline]
    pub fn compute_affine(&self) -> Affine3A {
        Affine3A::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Rotate this transform so that its local **-Z** axis points from its
    /// translation toward `target` (the engine's camera-forward convention),
    /// keeping `up` (usually +Y) as vertical as possible.
    ///
    /// Degenerate inputs (target coincides with the translation, or `up`
    /// parallel to the view direction) leave the rotation unchanged.
    pub fn look_at(&mut self, target: Vec3, up: Vec3) {
        let back = self.translation - target;
        if back.length_squared() < 1e-12 || up.length_squared() < 1e-12 {
            return;
        }
        let back = back.normalize();
        let right_raw = up.normalize().cross(back);
        if right_raw.length_squared() < 1e-12 {
            return; // up parallel to the view direction
        }
        let right = right_raw.normalize();
        let up = back.cross(right);
        self.rotation = Quat::from_mat3(&Mat3::from_cols(right, up, back));
    }

    /// `self` rotated by [`look_at`](Self::look_at) (builder style).
    #[inline]
    #[must_use]
    pub fn looking_at(mut self, target: Vec3, up: Vec3) -> Self {
        self.look_at(target, up);
        self
    }

    /// The [`Mat4`] of this transform.
    #[inline]
    pub fn compute_matrix(&self) -> Mat4 {
        Mat4::from(self.compute_affine())
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl From<Vec3> for Transform {
    fn from(translation: Vec3) -> Self {
        Self::from_translation(translation)
    }
}

/// A world-space transform: the composition of a [`Transform`] with all its
/// ancestors, maintained by the propagation system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalTransform(Affine3A);

impl GlobalTransform {
    /// The identity transform.
    pub const IDENTITY: Self = Self(Affine3A::IDENTITY);

    /// The world-space affine.
    #[inline]
    pub fn affine(&self) -> Affine3A {
        self.0
    }

    /// Replace the world-space affine. Used by the propagation system.
    #[inline]
    pub fn set_affine(&mut self, affine: Affine3A) {
        self.0 = affine;
    }

    /// The world-space translation.
    #[inline]
    pub fn translation(&self) -> Vec3 {
        Vec3::from(self.0.translation)
    }

    /// Transform a world-space point.
    #[inline]
    pub fn transform_point(&self, point: Vec3) -> Vec3 {
        self.0.transform_point3(point)
    }
}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl From<Affine3A> for GlobalTransform {
    fn from(affine: Affine3A) -> Self {
        Self(affine)
    }
}

impl From<Transform> for GlobalTransform {
    fn from(transform: Transform) -> Self {
        Self(transform.compute_affine())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_vec3_close(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn test_look_at_along_minus_z_is_identity() {
        // Camera 5 units up +Z looking at the origin already looks down -Z.
        let t = Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y);
        assert!((t.rotation - Quat::IDENTITY).length() < 1e-5);
    }

    #[test]
    fn test_look_at_points_forward_at_target() {
        let t = Transform::from_xyz(0.0, 1.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y);
        // Local -Z, transformed by the rotation, points at the target.
        let forward = t.rotation * Vec3::NEG_Z;
        let to_target = (Vec3::ZERO - t.translation).normalize();
        assert_vec3_close(forward, to_target);
        // Up stays roughly up (no roll).
        let rotated_up = t.rotation * Vec3::Y;
        assert!(rotated_up.y > 0.9);
    }

    #[test]
    fn test_look_at_degenerate_target_keeps_rotation() {
        let mut t = Transform::from_rotation(Quat::from_rotation_y(1.0));
        let before = t.rotation;
        t.look_at(t.translation, Vec3::Y); // target == translation
        t.look_at(Vec3::ZERO, Vec3::ZERO); // zero up
        assert_eq!(t.rotation, before);
    }
}

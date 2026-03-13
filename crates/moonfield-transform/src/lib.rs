//! Transform component for 3D transformations
//! 
//! This crate provides a Transform component that represents position, rotation, and scale
//! in 3D space. It provides utility methods for common transformation operations.

use moonfield_math::{Vec3 as Vector3, Quat as UnitQuaternion, Mat4 as Matrix4, Point3, nalgebra};

/// A component that represents the position, rotation, and scale of an entity in 3D space
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Position of the transform in world space
    pub translation: Vector3,
    /// Rotation of the transform as a quaternion
    pub rotation: UnitQuaternion,
    /// Scale of the transform
    pub scale: Vector3,
}

impl Transform {
    /// Create a new identity transform (position at origin, no rotation, scale of 1)
    pub fn identity() -> Self {
        Self {
            translation: Vector3::new(0.0, 0.0, 0.0),
            rotation: UnitQuaternion::identity(),
            scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }

    /// Create a new transform with the specified translation
    pub fn from_translation(translation: Vector3) -> Self {
        Self {
            translation,
            rotation: UnitQuaternion::identity(),
            scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }

    /// Create a new transform with the specified rotation
    pub fn from_rotation(rotation: UnitQuaternion) -> Self {
        Self {
            translation: Vector3::new(0.0, 0.0, 0.0),
            rotation,
            scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }

    /// Create a new transform with the specified scale
    pub fn from_scale(scale: Vector3) -> Self {
        Self {
            translation: Vector3::new(0.0, 0.0, 0.0),
            rotation: UnitQuaternion::identity(),
            scale,
        }
    }

    /// Create a new transform with the specified translation, rotation, and scale
    pub fn from_translation_rotation_scale(translation: Vector3, rotation: UnitQuaternion, scale: Vector3) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// Get the forward direction of the transform (in world space)
    pub fn forward(&self) -> Vector3 {
        self.rotation.transform_vector(&Vector3::new(0.0, 0.0, -1.0))
    }

    /// Get the backward direction of the transform (in world space)
    pub fn backward(&self) -> Vector3 {
        self.rotation.transform_vector(&Vector3::new(0.0, 0.0, 1.0))
    }

    /// Get the up direction of the transform (in world space)
    pub fn up(&self) -> Vector3 {
        self.rotation.transform_vector(&Vector3::new(0.0, 1.0, 0.0))
    }

    /// Get the down direction of the transform (in world space)
    pub fn down(&self) -> Vector3 {
        self.rotation.transform_vector(&Vector3::new(0.0, -1.0, 0.0))
    }

    /// Get the right direction of the transform (in world space)
    pub fn right(&self) -> Vector3 {
        self.rotation.transform_vector(&Vector3::new(1.0, 0.0, 0.0))
    }

    /// Get the left direction of the transform (in world space)
    pub fn left(&self) -> Vector3 {
        self.rotation.transform_vector(&Vector3::new(-1.0, 0.0, 0.0))
    }

    /// Translate the transform by the given offset
    pub fn translate(&mut self, offset: Vector3) {
        self.translation += offset;
    }

    /// Rotate the transform by the given quaternion
    pub fn rotate(&mut self, rotation: UnitQuaternion) {
        self.rotation = self.rotation * rotation;
        // Note: nalgebra quaternions stay normalized through multiplication, so no renormalization needed
    }

    /// Scale the transform uniformly
    pub fn scale_uniform(&mut self, factor: f32) {
        self.scale *= factor;
    }

    /// Scale the transform by the given scale vector
    pub fn scale_by(&mut self, scale: Vector3) {
        self.scale = self.scale.component_mul(&scale);
    }

    /// Set the translation of the transform
    pub fn set_translation(&mut self, translation: Vector3) {
        self.translation = translation;
    }

    /// Set the rotation of the transform
    pub fn set_rotation(&mut self, rotation: UnitQuaternion) {
        self.rotation = rotation;
    }

    /// Set the scale of the transform
    pub fn set_scale(&mut self, scale: Vector3) {
        self.scale = scale;
    }

    /// Create a look-at transform that rotates to look at the target
    pub fn look_at(eye: Point3, target: Point3, up: Vector3) -> Self {
        let forward = (target - eye).normalize();
        let right = up.cross(&forward).normalize();
        let up_new = forward.cross(&right).normalize();

        // Create rotation matrix from basis vectors
        let rotation_matrix = nalgebra::Rotation3::from_matrix_unchecked(
            nalgebra::Matrix3::from_columns(&[
                right,
                up_new,
                forward,
            ])
        );

        // Convert to quaternion
        let rotation = UnitQuaternion::from_rotation_matrix(&rotation_matrix);

        Self {
            translation: eye.coords,
            rotation,
            scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }

    /// Compute the transformation matrix for this transform
    pub fn compute_matrix(&self) -> Matrix4 {
        Matrix4::new_translation(&self.translation)
            * self.rotation.to_homogeneous()
            * Matrix4::new_nonuniform_scaling(&self.scale)
    }

    /// Compute the inverse transformation matrix for this transform
    pub fn compute_inverse_matrix(&self) -> Matrix4 {
        let inv_scale = Vector3::new(1.0 / self.scale.x, 1.0 / self.scale.y, 1.0 / self.scale.z);
        Matrix4::new_nonuniform_scaling(&inv_scale)
            * self.rotation.inverse().to_homogeneous()
            * Matrix4::new_translation(&(-self.translation))
    }

    /// Apply this transform to a point
    pub fn transform_point(&self, point: Point3) -> Point3 {
        let transformed = self.compute_matrix() * point.to_homogeneous();
        Point3::from_homogeneous(transformed).unwrap_or(Point3::origin())
    }

    /// Apply this transform to a vector (ignores translation)
    pub fn transform_vector(&self, vector: Vector3) -> Vector3 {
        self.rotation.transform_vector(&(vector.component_mul(&self.scale)))
    }

    /// Combine this transform with another transform
    pub fn combine(&self, other: &Self) -> Self {
        let combined_translation = self.transform_point(Point3::from(other.translation));
        let combined_rotation = self.rotation * other.rotation;
        let combined_scale = Vector3::new(
            self.scale.x * other.scale.x,
            self.scale.y * other.scale.y,
            self.scale.z * other.scale.z,
        );

        Self {
            translation: combined_translation.coords,
            rotation: combined_rotation,
            scale: combined_scale,
        }
    }

    /// Interpolate between two transforms
    pub fn lerp(start: &Self, end: &Self, t: f32) -> Self {
        let t_clamped = t.max(0.0).min(1.0);
        
        let translation = start.translation + (end.translation - start.translation) * t_clamped;
        let rotation = start.rotation.slerp(&end.rotation, t_clamped);
        let scale = start.scale + (end.scale - start.scale) * t_clamped;

        Self {
            translation,
            rotation,
            scale,
        }
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_transform() {
        let transform = Transform::identity();
        assert_eq!(transform.translation, Vector3::new(0.0, 0.0, 0.0));
        assert_eq!(transform.rotation, UnitQuaternion::identity());
        assert_eq!(transform.scale, Vector3::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn test_translation() {
        let mut transform = Transform::identity();
        let offset = Vector3::new(1.0, 2.0, 3.0);
        transform.translate(offset);
        assert_eq!(transform.translation, offset);
    }

    #[test]
    fn test_compute_matrix_identity() {
        let transform = Transform::identity();
        let matrix = transform.compute_matrix();
        
        // Identity transform should produce identity matrix
        let expected = Matrix4::identity();
        assert!((matrix - expected).norm() < 1e-5);
    }

    #[test]
    fn test_direction_functions() {
        let transform = Transform::identity();
        let forward = transform.forward();
        assert!((forward - Vector3::new(0.0, 0.0, -1.0)).norm() < 1e-5);
        
        let up = transform.up();
        assert!((up - Vector3::new(0.0, 1.0, 0.0)).norm() < 1e-5);
        
        let right = transform.right();
        assert!((right - Vector3::new(1.0, 0.0, 0.0)).norm() < 1e-5);
    }
}
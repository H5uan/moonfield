//! Scene-facing ECS components for rendering: [`Camera`] + [`PrimaryCamera`]
//! and the slice's renderable, [`MeshRenderer`].
//!
//! These are plain components (the blanket `Component` impl covers them); the
//! editor's viewport render path queries them directly from the world. The
//! projection uses the engine's single clip convention (Y-up, reverse-Z — see
//! [`crate::camera`]).

use moonfield_math::{GlobalTransform, Mat4};

use crate::camera::perspective_reverse_z;

/// A camera: projection parameters and the clear color for its target.
///
/// The camera's view transform comes from its entity's
/// [`GlobalTransform`] — the camera looks down its local **-Z** with **+Y**
/// up, per the engine's coordinate conventions.
#[derive(Debug, Clone, Copy, PartialEq, moonfield_reflect::Reflect)]
pub struct Camera {
    /// Vertical field of view in radians.
    pub fov_y_radians: f32,
    /// Near plane distance. There is no far plane: the projection is
    /// reverse-infinite-Z.
    pub near: f32,
    /// The color the camera's target is cleared to, linear RGBA.
    pub clear_color: [f32; 4],
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov_y_radians: std::f32::consts::FRAC_PI_3, // 60°
            near: 0.1,
            clear_color: [0.02, 0.02, 0.03, 1.0],
        }
    }
}

impl Camera {
    /// The reverse-Z projection matrix for a render target with the given
    /// aspect ratio (`width / height`).
    #[must_use]
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        perspective_reverse_z(self.fov_y_radians, aspect, self.near, f32::INFINITY)
    }
}

/// The view matrix for a camera whose world transform is `camera_global`:
/// the inverse of the camera's global transform.
#[must_use]
pub fn view_matrix(camera_global: &GlobalTransform) -> Mat4 {
    Mat4::from(camera_global.affine().inverse())
}

/// Marks the camera the editor viewport renders from.
///
/// Single-window, single-viewport convention: if several entities carry it,
/// the first one found wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrimaryCamera;

/// Renders the entity as a colored unit cube (1×1×1, centered on the origin),
/// transformed by its [`GlobalTransform`].
///
/// The mesh is the scene renderer's shared cube until the asset system
/// (roadmap milestone 8) provides real mesh handles; the flat color stands in
/// for a material.
#[derive(Debug, Clone, Copy, PartialEq, moonfield_reflect::Reflect)]
pub struct MeshRenderer {
    /// The cube's flat color, linear RGBA.
    pub color: [f32; 4],
}

impl MeshRenderer {
    /// A cube of the given color.
    pub fn colored(color: [f32; 4]) -> Self {
        Self { color }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_math::{Quat, Vec3};

    fn assert_vec3_close(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-4, "{a} != {b}");
    }

    #[test]
    fn test_camera_projection_is_reverse_z_y_up() {
        let camera = Camera::default();
        let proj = camera.projection_matrix(16.0 / 9.0);
        // Near plane maps to depth 1 (reverse-Z), far tends to 0.
        let near_z = proj.project_point3(Vec3::new(0.0, 0.0, -camera.near)).z;
        assert!((near_z - 1.0).abs() < 1e-5);
        // +Y in view space lands at positive NDC Y (engine Y-up convention).
        assert!(proj.project_point3(Vec3::new(0.0, 1.0, -5.0)).y > 0.0);
    }

    #[test]
    fn test_view_matrix_looks_down_local_minus_z() {
        // Camera at (0, 1, 5), unrotated (looking down world -Z).
        let mut global = GlobalTransform::from(moonfield_math::Transform::from_xyz(0.0, 1.0, 5.0));
        let view = view_matrix(&global);
        // The camera origin maps to view-space origin; a point 3 units in
        // front of the camera (world z = 2) lands at view z = -3.
        assert_vec3_close(view.transform_point3(Vec3::new(0.0, 1.0, 5.0)), Vec3::ZERO);
        assert_vec3_close(
            view.transform_point3(Vec3::new(0.0, 1.0, 2.0)),
            Vec3::new(0.0, 0.0, -3.0),
        );

        // Rotated -90° about Y: world +X is now in front of the camera.
        let rotated = moonfield_math::Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        };
        global = GlobalTransform::from(rotated);
        let view = view_matrix(&global);
        assert_vec3_close(
            view.transform_point3(Vec3::new(3.0, 0.0, 0.0)),
            Vec3::new(0.0, 0.0, -3.0),
        );
    }
}

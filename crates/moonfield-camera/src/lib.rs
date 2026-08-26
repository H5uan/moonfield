//! Scene-facing camera components and projection math.
//!
//! This crate owns camera data shared by scene, editor, and rendering crates.
//! It has no dependency on the Vulkan RHI.

use moonfield_math::{camera, GlobalTransform, Mat4};

/// Right-handed reverse infinite-Z perspective projection with Moonfield's
/// clip convention: the camera looks down -Z, NDC Y points up, and depth maps
/// the near plane to 1 while tending toward 0 at infinity.
///
/// `far` is accepted for API symmetry and ignored because the projection has
/// no far clip plane.
#[must_use]
pub fn perspective_reverse_z(fov_y_radians: f32, aspect: f32, near: f32, _far: f32) -> Mat4 {
    camera::rh::proj::directx::perspective_infinite_reverse(fov_y_radians, aspect, near)
}

/// A camera's projection parameters and target clear color.
///
/// The view transform comes from the entity's [`GlobalTransform`]. A camera
/// looks down its local -Z axis with +Y up.
#[derive(Debug, Clone, Copy, PartialEq, moonfield_reflect::Reflect)]
pub struct Camera {
    /// Vertical field of view in radians.
    pub fov_y_radians: f32,
    /// Near plane distance. The projection has no far plane.
    pub near: f32,
    /// Target clear color in linear RGBA.
    pub clear_color: [f32; 4],
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            fov_y_radians: std::f32::consts::FRAC_PI_3,
            near: 0.1,
            clear_color: [0.02, 0.02, 0.03, 1.0],
        }
    }
}

impl Camera {
    /// Returns the reverse-Z projection matrix for `width / height`.
    #[must_use]
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        perspective_reverse_z(self.fov_y_radians, aspect, self.near, f32::INFINITY)
    }
}

/// Logical destination of a camera before a backend resolves GPU attachments.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RenderTarget {
    /// The application's primary presentation window.
    PrimaryWindow,
    /// The editor's offscreen viewport texture.
    #[default]
    Viewport,
}

/// Optional runtime target override attached beside [`Camera`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CameraTarget(pub RenderTarget);

/// Marks the camera used by the primary editor viewport.
///
/// If several entities carry this marker, the first one found wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrimaryCamera;

/// Returns the inverse of a camera's global transform.
#[must_use]
pub fn view_matrix(camera_global: &GlobalTransform) -> Mat4 {
    Mat4::from(camera_global.affine().inverse())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_math::{Quat, Transform, Vec3};

    const NEAR: f32 = 0.1;
    const FAR: f32 = 100.0;

    fn projection() -> Mat4 {
        perspective_reverse_z(std::f32::consts::FRAC_PI_2, 1.0, NEAR, FAR)
    }

    fn ndc(point: Vec3) -> Vec3 {
        projection().project_point3(point)
    }

    fn assert_vec3_close(left: Vec3, right: Vec3) {
        assert!((left - right).length() < 1e-4, "{left} != {right}");
    }

    #[test]
    fn test_reverse_z_near_maps_to_one() {
        let near_z = ndc(Vec3::new(0.0, 0.0, -NEAR)).z;
        assert!((near_z - 1.0).abs() < 1e-5, "near z = {near_z}");
    }

    #[test]
    fn test_reverse_z_far_tends_to_zero() {
        let far_z = ndc(Vec3::new(0.0, 0.0, -FAR)).z;
        assert!(far_z > 0.0 && far_z < 1.0, "far z = {far_z}");
        let deeper = ndc(Vec3::new(0.0, 0.0, -1e6)).z;
        assert!(
            deeper < far_z,
            "deeper z = {deeper} should be < far z = {far_z}"
        );
    }

    #[test]
    fn test_view_center_maps_to_ndc_origin() {
        let point = ndc(Vec3::new(0.0, 0.0, -5.0));
        assert!(point.x.abs() < 1e-5, "x = {}", point.x);
        assert!(point.y.abs() < 1e-5, "y = {}", point.y);
    }

    #[test]
    fn test_y_up() {
        assert!(ndc(Vec3::new(0.0, 1.0, -5.0)).y > 0.0);
        assert!(ndc(Vec3::new(0.0, -1.0, -5.0)).y < 0.0);
    }

    #[test]
    fn test_camera_projection_is_reverse_z_y_up() {
        let camera = Camera::default();
        let projection = camera.projection_matrix(16.0 / 9.0);
        let near_z = projection
            .project_point3(Vec3::new(0.0, 0.0, -camera.near))
            .z;
        assert!((near_z - 1.0).abs() < 1e-5);
        assert!(projection.project_point3(Vec3::new(0.0, 1.0, -5.0)).y > 0.0);
    }

    #[test]
    fn test_view_matrix_looks_down_local_minus_z() {
        let mut global = GlobalTransform::from(Transform::from_xyz(0.0, 1.0, 5.0));
        let view = view_matrix(&global);
        assert_vec3_close(view.transform_point3(Vec3::new(0.0, 1.0, 5.0)), Vec3::ZERO);
        assert_vec3_close(
            view.transform_point3(Vec3::new(0.0, 1.0, 2.0)),
            Vec3::new(0.0, 0.0, -3.0),
        );

        global = GlobalTransform::from(Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::ONE,
        });
        let view = view_matrix(&global);
        assert_vec3_close(
            view.transform_point3(Vec3::new(3.0, 0.0, 0.0)),
            Vec3::new(0.0, 0.0, -3.0),
        );
    }
}

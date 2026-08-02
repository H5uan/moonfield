//! Perspective camera producing view/projection matrices.
//!
//! The projection convention depends on the crate's backend feature:
//! `native` uses Vulkan conventions (NDC Y points down), `web` uses wgpu
//! conventions (NDC Y points up). Both map depth to [0, 1].

use moonfield_math::{Mat4, Vec3};

/// A perspective camera positioned by `position` and oriented by `yaw` /
/// `pitch` (radians). `yaw = 0, pitch = 0` looks down -Z, positive yaw turns
/// toward +X, positive pitch looks up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    /// Rotation around the world Y axis, radians.
    pub yaw: f32,
    /// Rotation around the camera's local X axis, radians.
    pub pitch: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// Viewport width / height.
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    /// Unit vector the camera is looking along, in world space.
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    /// Right-handed view matrix (camera looks down -Z in view space).
    #[must_use]
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.position + self.forward(), Vec3::Y)
    }

    /// Perspective projection. The convention is backend-dependent: with the
    /// `native` feature, Vulkan conventions (NDC depth in [0, 1], NDC Y
    /// points down); with the `web` feature, wgpu conventions (NDC depth in
    /// [0, 1], NDC Y points up).
    #[must_use]
    pub fn projection(&self) -> Mat4 {
        #[cfg(feature = "native")]
        {
            moonfield_math::projection::perspective_vk(self.fov_y, self.aspect, self.near, self.far)
        }
        #[cfg(feature = "web")]
        {
            moonfield_math::projection::perspective_wgpu(
                self.fov_y,
                self.aspect,
                self.near,
                self.far,
            )
        }
    }

    /// Combined `projection * view` matrix.
    #[must_use]
    pub fn view_projection(&self) -> Mat4 {
        self.projection() * self.view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_camera() -> Camera {
        Camera {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: std::f32::consts::FRAC_PI_2,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        }
    }

    fn ndc(cam: &Camera, p: Vec3) -> Vec3 {
        cam.view_projection().project_point3(p)
    }

    #[test]
    fn test_default_forward_is_negative_z() {
        let f = test_camera().forward();
        assert!(f.abs_diff_eq(Vec3::NEG_Z, 1e-6), "forward = {f}");
    }

    #[test]
    fn test_point_ahead_projects_to_ndc_center() {
        let cam = test_camera();
        let p = ndc(&cam, Vec3::new(0.0, 0.0, -5.0));
        assert!(p.x.abs() < 1e-5, "x = {}", p.x);
        assert!(p.y.abs() < 1e-5, "y = {}", p.y);
        assert!((0.0..=1.0).contains(&p.z), "z = {}", p.z);
    }

    #[test]
    fn test_near_and_far_map_to_zero_and_one() {
        let cam = test_camera();
        let near_z = ndc(&cam, Vec3::new(0.0, 0.0, -cam.near)).z;
        let far_z = ndc(&cam, Vec3::new(0.0, 0.0, -cam.far)).z;
        assert!(near_z.abs() < 1e-5, "near z = {near_z}");
        assert!((far_z - 1.0).abs() < 1e-5, "far z = {far_z}");
    }

    #[test]
    #[cfg(feature = "native")]
    fn test_vulkan_y_flip() {
        // A point above the camera axis (+Y in world) must land at negative
        // NDC Y, because Vulkan NDC Y points down.
        let cam = test_camera();
        let y = ndc(&cam, Vec3::new(0.0, 1.0, -5.0)).y;
        assert!(y < 0.0, "y = {y}");
    }

    #[test]
    #[cfg(feature = "web")]
    fn test_wgpu_y_up() {
        // A point above the camera axis (+Y in world) must land at positive
        // NDC Y, because wgpu NDC Y points up.
        let cam = test_camera();
        let y = ndc(&cam, Vec3::new(0.0, 1.0, -5.0)).y;
        assert!(y > 0.0, "y = {y}");
    }
}

//! Perspective camera producing view/projection matrices.
//!
//! Moonfield uses a single engine projection convention: Y-up NDC with
//! reverse-Z depth (`far -> 0`). Vulkan-specific viewport adjustments are
//! applied at the renderer boundary, never in this shared matrix.

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

    /// Perspective projection using moonfield's single convention: reverse-Z,
    /// Y-up NDC. Backend-specific NDC transforms are applied at the backend
    /// boundary, not here.
    #[must_use]
    pub fn projection(&self) -> Mat4 {
        moonfield_render::perspective_reverse_z(self.fov_y, self.aspect, self.near, self.far)
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
    fn test_reverse_z_near_maps_to_one() {
        let cam = test_camera();
        // Reverse-Z: near plane maps to depth 1 (not 0).
        let near_z = ndc(&cam, Vec3::new(0.0, 0.0, -cam.near)).z;
        assert!((near_z - 1.0).abs() < 1e-5, "near z = {near_z}");
    }

    #[test]
    fn test_reverse_z_far_tends_to_zero() {
        let cam = test_camera();
        // Reverse-Z: depth decreases toward far; a large distance is near 0.
        let far_z = ndc(&cam, Vec3::new(0.0, 0.0, -cam.far)).z;
        assert!(far_z > 0.0 && far_z < 1.0, "far z = {far_z}");
        let deeper = ndc(&cam, Vec3::new(0.0, 0.0, -1e6)).z;
        assert!(deeper < far_z, "deeper z = {deeper}");
    }

    #[test]
    fn test_y_up() {
        // A point above the camera axis (+Y in world) lands at positive NDC Y,
        // because the engine convention is Y-up.
        let cam = test_camera();
        let y = ndc(&cam, Vec3::new(0.0, 1.0, -5.0)).y;
        assert!(y > 0.0, "y = {y}");
        let y = ndc(&cam, Vec3::new(0.0, -1.0, -5.0)).y;
        assert!(y < 0.0, "y = {y}");
    }
}

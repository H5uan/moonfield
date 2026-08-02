//! Projection helpers encoding moonfield's clip-space conventions.
//!
//! Vulkan clip/NDC space has Y pointing *down* and depth in `[0, 1]`, unlike
//! OpenGL's Y-up, `[-1, 1]` conventions. All projection matrices in the
//! workspace must come from this module so the convention lives in exactly
//! one place — do not hand-assemble projection matrices elsewhere.
//!
//! View matrices need no wrapper: glam's [`Mat4::look_at_rh`] already matches
//! moonfield's right-handed, Y-up, camera-looks-down-`-Z` convention, so call
//! it directly.

use crate::Mat4;

/// Right-handed perspective projection with Vulkan clip-space conventions:
/// the camera looks down -Z, NDC depth maps `near -> 0` and `far -> 1`, and
/// the Y axis is flipped because Vulkan NDC Y points down.
///
/// Built on [`Mat4::perspective_rh`] (right-handed, zero-to-one depth; the
/// OpenGL-style `[-1, 1]` variant is `perspective_rh_gl`) with `y_axis.y`
/// negated for the Vulkan Y flip.
///
/// Note: this may switch to a reverse infinite-Z projection (the approach
/// Bevy uses) in the future; when that happens, only this function changes
/// and every caller picks the new convention up for free.
#[must_use]
pub fn perspective_vk(fov_y_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let mut proj = Mat4::perspective_rh(fov_y_radians, aspect, near, far);
    proj.y_axis.y = -proj.y_axis.y;
    proj
}

/// Right-handed perspective projection with wgpu/WebGPU clip-space
/// conventions: the camera looks down -Z, NDC depth maps `near -> 0` and
/// `far -> 1`, and NDC Y points *up* (the D3D/Metal convention wgpu
/// follows). This is exactly [`Mat4::perspective_rh`] — no Y flip.
#[must_use]
pub fn perspective_wgpu(fov_y_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    Mat4::perspective_rh(fov_y_radians, aspect, near, far)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;

    const NEAR: f32 = 0.1;
    const FAR: f32 = 100.0;

    fn proj() -> Mat4 {
        perspective_vk(std::f32::consts::FRAC_PI_2, 1.0, NEAR, FAR)
    }

    /// NDC coordinates of a view-space point (camera at origin, looking -Z).
    fn ndc(p: Vec3) -> Vec3 {
        proj().project_point3(p)
    }

    #[test]
    fn test_near_and_far_map_to_zero_and_one_depth() {
        let near_z = ndc(Vec3::new(0.0, 0.0, -NEAR)).z;
        let far_z = ndc(Vec3::new(0.0, 0.0, -FAR)).z;
        assert!(near_z.abs() < 1e-5, "near z = {near_z}");
        assert!((far_z - 1.0).abs() < 1e-5, "far z = {far_z}");
    }

    #[test]
    fn test_view_center_maps_to_ndc_origin() {
        let p = ndc(Vec3::new(0.0, 0.0, -5.0));
        assert!(p.x.abs() < 1e-5, "x = {}", p.x);
        assert!(p.y.abs() < 1e-5, "y = {}", p.y);
    }

    #[test]
    fn test_vulkan_y_flip() {
        // A point above the camera axis (+Y in view space) must land at
        // negative NDC Y, because Vulkan NDC Y points down.
        let y = ndc(Vec3::new(0.0, 1.0, -5.0)).y;
        assert!(y < 0.0, "y = {y}");
        // Symmetrically, a point below lands at positive NDC Y.
        let y = ndc(Vec3::new(0.0, -1.0, -5.0)).y;
        assert!(y > 0.0, "y = {y}");
    }

    #[test]
    fn test_wgpu_y_up() {
        let proj = perspective_wgpu(std::f32::consts::FRAC_PI_2, 1.0, NEAR, FAR);
        // Depth convention matches Vulkan: near -> 0, far -> 1.
        let near_z = proj.project_point3(Vec3::new(0.0, 0.0, -NEAR)).z;
        let far_z = proj.project_point3(Vec3::new(0.0, 0.0, -FAR)).z;
        assert!(near_z.abs() < 1e-5, "near z = {near_z}");
        assert!((far_z - 1.0).abs() < 1e-5, "far z = {far_z}");
        // But NDC Y points up: a point above the camera axis lands at
        // positive NDC Y, and vice versa.
        let y = proj.project_point3(Vec3::new(0.0, 1.0, -5.0)).y;
        assert!(y > 0.0, "y = {y}");
        let y = proj.project_point3(Vec3::new(0.0, -1.0, -5.0)).y;
        assert!(y < 0.0, "y = {y}");
    }
}

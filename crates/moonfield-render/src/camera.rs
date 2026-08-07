//! Projection helpers encoding moonfield's single clip-space convention.
//!
//! Moonfield uses **one** NDC convention across every backend: Y points *up*
//! and depth is **reverse** (`far -> 0`, near -> 1) — the wgpu / WebGPU / Bevy
//! convention. There is no per-backend projection matrix; if a particular
//! backend (e.g. Vulkan via `ash`) needs a Y-flip or a different depth swizzle,
//! that is applied as an *extra transform at the backend boundary*, never by
//! swapping the projection matrix itself.
//!
//! Projection construction is a camera concern, so it lives in the render
//! crate (mirroring `bevy_render::camera`), not in `moonfield-math`.

use moonfield_math::Mat4;

/// Right-handed **reverse infinite-Z** perspective projection with moonfield's
/// single clip convention: the camera looks down -Z, NDC Y points *up*, and
/// depth is reversed — `near -> 1`, `far -> -infinity` (clipped to 0).
///
/// Built on [`Mat4::perspective_infinite_reverse_rh`]. This is the convention
/// shared by wgpu / WebGPU / Bevy and is the *only* projection matrix moonfield
/// produces; backends that need a different NDC (Vulkan's Y-down) add an extra
/// transform at their boundary instead of assembling their own matrix.
///
/// # Infinite far plane
///
/// The reverse-infinite projection has no far clip plane; `far` is accepted
/// only for API symmetry and is ignored. Depth precision is concentrated near
/// the camera, which is where it matters most.
#[must_use]
pub fn perspective_reverse_z(fov_y_radians: f32, aspect: f32, near: f32, _far: f32) -> Mat4 {
    Mat4::perspective_infinite_reverse_rh(fov_y_radians, aspect, near)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_math::Vec3;

    const NEAR: f32 = 0.1;
    const FAR: f32 = 100.0;

    fn proj() -> Mat4 {
        perspective_reverse_z(std::f32::consts::FRAC_PI_2, 1.0, NEAR, FAR)
    }

    /// NDC coordinates of a view-space point (camera at origin, looking -Z).
    fn ndc(p: Vec3) -> Vec3 {
        proj().project_point3(p)
    }

    #[test]
    fn test_reverse_z_near_maps_to_one() {
        // Reverse-Z: the near plane maps to depth 1, not 0.
        let near_z = ndc(Vec3::new(0.0, 0.0, -NEAR)).z;
        assert!((near_z - 1.0).abs() < 1e-5, "near z = {near_z}");
    }

    #[test]
    fn test_reverse_z_far_tends_to_zero() {
        // Reverse-Z: depth decreases toward far; a large distance is near 0.
        let far_z = ndc(Vec3::new(0.0, 0.0, -FAR)).z;
        assert!(far_z > 0.0 && far_z < 1.0, "far z = {far_z}");
        let deeper = ndc(Vec3::new(0.0, 0.0, -1e6)).z;
        assert!(deeper < far_z, "deeper z = {deeper} should be < far z = {far_z}");
    }

    #[test]
    fn test_view_center_maps_to_ndc_origin() {
        let p = ndc(Vec3::new(0.0, 0.0, -5.0));
        assert!(p.x.abs() < 1e-5, "x = {}", p.x);
        assert!(p.y.abs() < 1e-5, "y = {}", p.y);
    }

    #[test]
    fn test_y_up() {
        // A point above the camera axis (+Y in view space) lands at positive
        // NDC Y, because the convention is wgpu-style (Y up).
        let y = ndc(Vec3::new(0.0, 1.0, -5.0)).y;
        assert!(y > 0.0, "y = {y}");
        let y = ndc(Vec3::new(0.0, -1.0, -5.0)).y;
        assert!(y < 0.0, "y = {y}");
    }
}
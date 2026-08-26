//! Viewport interaction math: the orbit camera, screen ⇄ world conversion,
//! and gizmo hit-testing / dragging.
//!
//! Everything here is pure math on egui screen coordinates and engine math
//! types — no Vulkan, no world access — so the whole module is unit-testable
//! headless. The world-touching glue (reading the selection, writing back
//! local [`Transform`]s) lives in `ui.rs`.
//!
//! Coordinate conventions: the engine's clip space is Y-up reverse-Z (near
//! plane → NDC z = 1, see `moonfield-math`'s module docs), and the viewport's
//! Vulkan pass flips Y with a negative-height viewport, so screen space here
//! is egui's top-left-origin space. Both quirks are confined to
//! [`world_to_screen`] / [`screen_to_ray`].

use moonfield_math::{Affine3A, Dir3, EulerRot, Mat4, Quat, Ray3d, Transform, Vec3};

/// On-screen length of gizmo axis handles, in egui points.
pub const GIZMO_SCREEN_LENGTH: f32 = 64.0;
/// On-screen radius of gizmo rotation rings, in egui points.
pub const GIZMO_RING_RADIUS: f32 = 56.0;
/// Pointer distance within which a gizmo handle counts as hit, in points.
pub const GIZMO_HIT_RADIUS: f32 = 8.0;

const MIN_PITCH: f32 = -std::f32::consts::FRAC_PI_2 + 0.01;
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;
const MIN_DISTANCE: f32 = 0.05;
const MAX_DISTANCE: f32 = 10_000.0;
const MIN_SCALE: f32 = 0.01;

/// The viewport's editor-controlled orbit camera.
///
/// The camera pose is owned by the editor, not the scene: every frame the
/// editor writes [`OrbitCamera::transform`] into the `PrimaryCamera` entity's
/// `Transform`. The pivot is the point the camera orbits and zooms toward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCamera {
    /// The world-space point the camera looks at and orbits around.
    pub pivot: Vec3,
    /// Rotation around the world Y axis, in radians.
    pub yaw: f32,
    /// Rotation around the camera's local X axis, in radians.
    pub pitch: f32,
    /// Distance from the pivot.
    pub distance: f32,
}

impl OrbitCamera {
    /// Reconstruct an orbit camera from an existing camera transform: the
    /// pivot sits `distance` ahead along the view direction, with the
    /// distance guessed from the transform's offset from the world origin.
    pub fn from_transform(transform: &Transform) -> Self {
        let distance = transform.translation.length().clamp(1.0, MAX_DISTANCE);
        let pivot = transform.translation + transform.rotation * Vec3::NEG_Z * distance;
        let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        Self {
            pivot,
            yaw,
            pitch,
            distance,
        }
    }

    /// The camera pose as a [`Transform`] (camera looks down its local -Z).
    pub fn transform(&self) -> Transform {
        let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        Transform::from_translation(self.pivot + rotation * Vec3::Z * self.distance)
            .looking_at(self.pivot, Vec3::Y)
    }

    /// Orbit around the pivot by a screen-space drag delta (points).
    pub fn orbit(&mut self, delta: egui::Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch - delta.y * 0.01).clamp(MIN_PITCH, MAX_PITCH);
    }

    /// Pan the pivot by a screen-space drag delta (points), scaled with
    /// distance so panning feels uniform at any zoom.
    pub fn pan(&mut self, delta: egui::Vec2) {
        let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
        let speed = self.distance * 0.002;
        self.pivot += (rotation * Vec3::X * -delta.x + rotation * Vec3::Y * delta.y) * speed;
    }

    /// Dolly toward/away from the pivot following the scroll delta.
    pub fn zoom(&mut self, scroll_delta: f32) {
        self.distance =
            (self.distance * (1.0 - scroll_delta * 0.001)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }
}

/// Project a world-space point to egui screen space (top-left origin).
///
/// Returns `None` when the point is behind the camera. The Y-flip performed
/// by the Vulkan negative-height viewport is undone here, so the result lines
/// up with the egui image of the offscreen target.
pub fn world_to_screen(point: Vec3, view_proj: Mat4, rect: egui::Rect) -> Option<egui::Pos2> {
    let clip = view_proj * point.extend(1.0);
    if clip.w <= 0.0 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    Some(egui::pos2(
        rect.min.x + (ndc.x + 1.0) * 0.5 * rect.width(),
        rect.min.y + (1.0 - ndc.y) * 0.5 * rect.height(),
    ))
}

/// Unproject an egui screen position to a world-space picking ray.
///
/// The ray origin sits on the near plane (reverse-Z: NDC z = 1); the
/// direction is taken from a second unprojected point halfway down the depth
/// range, which stays finite under the infinite reverse-Z projection.
/// Returns `None` when the view-projection is singular or the direction
/// degenerates.
pub fn screen_to_ray(screen: egui::Pos2, rect: egui::Rect, view_proj: Mat4) -> Option<Ray3d> {
    let inverse = view_proj.inverse();
    let ndc = Vec3::new(
        (screen.x - rect.min.x) / rect.width().max(1.0) * 2.0 - 1.0,
        1.0 - (screen.y - rect.min.y) / rect.height().max(1.0) * 2.0,
        1.0,
    );
    let near = inverse.project_point3(ndc);
    let mid = inverse.project_point3(Vec3::new(ndc.x, ndc.y, 0.5));
    let direction = Dir3::new(mid - near).ok()?;
    Some(Ray3d::new(near, direction))
}

/// A world-space TRS: (translation, rotation, scale). Note this differs
/// from glam's `to_scale_rotation_translation` return order — callers
/// reorder explicitly at that boundary (see `ui.rs`).
pub type WorldTrs = (Vec3, Quat, Vec3);

/// Which gizmo operation the viewport handle performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoMode {
    /// Drag along an axis to move the entity.
    Translate,
    /// Drag a ring to rotate the entity around an axis.
    Rotate,
    /// Drag an axis (or the center for uniform) to scale the entity.
    Scale,
}

/// One grabbable gizmo part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoHandle {
    /// The X (0), Y (1), or Z (2) axis handle.
    Axis(usize),
    /// The center handle (uniform scale only).
    Uniform,
}

/// The gizmo's placement for one frame: the selected entity's world-space
/// pivot and unit axis directions (its world rotation, i.e. local-mode
/// gizmo).
#[derive(Debug, Clone, Copy)]
pub struct GizmoFrame {
    /// World-space pivot the gizmo is centered on.
    pub origin: Vec3,
    /// World-space unit directions of the entity's local X/Y/Z axes.
    pub axes: [Vec3; 3],
}

impl GizmoFrame {
    /// Build the frame from an entity's world position and rotation.
    pub fn new(origin: Vec3, rotation: Quat) -> Self {
        Self {
            origin,
            axes: [
                (rotation * Vec3::X).normalize_or_zero(),
                (rotation * Vec3::Y).normalize_or_zero(),
                (rotation * Vec3::Z).normalize_or_zero(),
            ],
        }
    }

    /// The world-per-screen-point scale at the gizmo origin, measured along
    /// `dir`. `None` when either projection fails or the direction projects
    /// to nothing (points straight at the camera).
    fn px_per_unit(&self, dir: Vec3, view_proj: Mat4, rect: egui::Rect) -> Option<f32> {
        let s0 = world_to_screen(self.origin, view_proj, rect)?;
        let s1 = world_to_screen(self.origin + dir, view_proj, rect)?;
        let ppu = (s1 - s0).length();
        (ppu > 1e-3).then_some(ppu)
    }

    /// The screen-space start/end of axis `axis`'s handle.
    ///
    /// Gizmo handles are a screen-space construct: the axis direction comes
    /// from projecting a unit step, and the endpoint is then placed at
    /// exactly [`GIZMO_SCREEN_LENGTH`] points along that 2D direction. This
    /// keeps the handle the same size at any distance — unlike scaling a
    /// world-space length, which perspective foreshortening makes
    /// distance-dependent for any axis not perpendicular to the view.
    /// `None` when the origin is behind the camera or the axis projects to
    /// a point (pointing straight at the camera).
    pub fn axis_segment(
        &self,
        axis: usize,
        view_proj: Mat4,
        rect: egui::Rect,
    ) -> Option<(egui::Pos2, egui::Pos2)> {
        let start = world_to_screen(self.origin, view_proj, rect)?;
        let dir = world_to_screen(self.origin + self.axes[axis], view_proj, rect)? - start;
        if dir.length_sq() < 1e-6 {
            return None;
        }
        Some((start, start + dir.normalized() * GIZMO_SCREEN_LENGTH))
    }

    /// A screen-space polyline of the rotation ring around axis `axis`
    /// (the circle in the plane through the origin perpendicular to it).
    ///
    /// The world radius is derived from the least-foreshortened of the
    /// ring's two basis directions, so the ring's widest extent stays near
    /// [`GIZMO_RING_RADIUS`] points regardless of orientation.
    pub fn ring_points(&self, axis: usize, view_proj: Mat4, rect: egui::Rect) -> Vec<egui::Pos2> {
        let normal = self.axes[axis];
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        let ppu = [u, v]
            .iter()
            .filter_map(|d| self.px_per_unit(*d, view_proj, rect))
            .fold(0.0f32, f32::max);
        if ppu <= 0.0 {
            return Vec::new();
        }
        let radius = GIZMO_RING_RADIUS / ppu;
        const SEGMENTS: usize = 48;
        (0..=SEGMENTS)
            .filter_map(|i| {
                let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                let point = self.origin + (u * angle.cos() + v * angle.sin()) * radius;
                world_to_screen(point, view_proj, rect)
            })
            .collect()
    }
}

/// Find the gizmo handle under the pointer, if any. Axis handles win over
/// the center handle; among axes the closest one wins.
pub fn hit_test(
    mode: GizmoMode,
    frame: &GizmoFrame,
    view_proj: Mat4,
    rect: egui::Rect,
    pointer: egui::Pos2,
) -> Option<GizmoHandle> {
    let mut best: Option<(f32, GizmoHandle)> = None;
    let mut consider = |distance: f32, handle: GizmoHandle| {
        if distance <= GIZMO_HIT_RADIUS && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, handle));
        }
    };

    match mode {
        GizmoMode::Translate | GizmoMode::Scale => {
            for axis in 0..3 {
                if let Some((start, end)) = frame.axis_segment(axis, view_proj, rect) {
                    consider(
                        point_segment_distance(pointer, start, end),
                        GizmoHandle::Axis(axis),
                    );
                }
            }
            if mode == GizmoMode::Scale {
                if let Some(center) = world_to_screen(frame.origin, view_proj, rect) {
                    consider(center.distance(pointer) - 6.0, GizmoHandle::Uniform);
                }
            }
        }
        GizmoMode::Rotate => {
            for axis in 0..3 {
                let points = frame.ring_points(axis, view_proj, rect);
                for pair in points.windows(2) {
                    consider(
                        point_segment_distance(pointer, pair[0], pair[1]),
                        GizmoHandle::Axis(axis),
                    );
                }
            }
        }
    }
    best.map(|(_, handle)| handle)
}

/// Distance from a point to a screen-space line segment.
fn point_segment_distance(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// The point where the ray crosses the plane through `origin` with the
/// given `normal`. `None` when parallel or behind the ray origin.
fn ray_plane_point(ray: Ray3d, origin: Vec3, normal: Vec3) -> Option<Vec3> {
    let denom = ray.direction.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = (origin - ray.origin).dot(normal) / denom;
    (t > 0.0).then(|| ray.at(t))
}

/// The plane used for axis translation drags: contains the axis and faces
/// the drag ray as directly as possible (normal perpendicular to the axis,
/// in the plane spanned by the axis and the ray direction).
///
/// Unlike a closest-point-between-lines approach this stays stable when the
/// view ray is nearly parallel to the axis — the classic "drag the axis
/// pointing at the camera and the entity teleports" failure. It degenerates
/// only when the axis is exactly parallel to the ray (the handle projects
/// to a point anyway), which returns `None`.
fn axis_drag_plane(axis: Vec3, ray_dir: Vec3) -> Option<Vec3> {
    (axis.cross(ray_dir).cross(axis)).try_normalize()
}

/// The signed angle rotating `from` to `to` around `axis`.
fn signed_angle(from: Vec3, to: Vec3, axis: Vec3) -> f32 {
    from.cross(to).dot(axis).atan2(from.dot(to))
}

/// An in-progress gizmo drag: the mode-specific snapshot taken when the
/// pointer went down on a handle. Applying a drag produces a new world-space
/// TRS; converting it to the entity's local [`Transform`] is the caller's
/// job (see [`world_trs_to_local`]).
#[derive(Debug, Clone, Copy)]
pub enum GizmoDrag {
    /// Translate along an axis.
    Translate {
        /// The dragged axis index (for highlighting).
        axis: usize,
        /// World-space unit direction of the dragged axis, frozen at drag
        /// start so later origin/rotation changes can't feed back.
        axis_dir: Vec3,
        /// World-space pivot at drag start (the drag plane passes through
        /// it).
        origin: Vec3,
        /// Normal of the drag plane (see [`axis_drag_plane`]), frozen at
        /// drag start so begin/apply measure against the same plane.
        plane_normal: Vec3,
        /// Signed axis offset of the grab point: `(point - origin) · axis`
        /// at drag start.
        start_offset: f32,
        /// World-space translation at drag start.
        start_translation: Vec3,
        /// World-space rotation (pass-through for the result TRS).
        rotation: Quat,
        /// World-space scale (pass-through for the result TRS).
        scale: Vec3,
    },
    /// Rotate around an axis.
    Rotate {
        /// The dragged axis index (for highlighting).
        axis: usize,
        /// World-space unit direction of the dragged axis, frozen at drag
        /// start (the entity rotates while dragging, so its live axes
        /// would feed back).
        axis_dir: Vec3,
        /// World-space pivot at drag start.
        origin: Vec3,
        /// In-plane direction from the origin to the grab point.
        start_vec: Vec3,
        /// World-space translation (pass-through for the result TRS).
        translation: Vec3,
        /// World-space rotation at drag start.
        start_rotation: Quat,
        /// World-space scale (pass-through for the result TRS).
        scale: Vec3,
    },
    /// Scale along an axis or uniformly.
    Scale {
        /// The handle being dragged.
        handle: GizmoHandle,
        /// Pointer distance from the gizmo center at drag start, in points.
        start_distance: f32,
        /// World-space translation (pass-through for the result TRS).
        translation: Vec3,
        /// World-space rotation (pass-through for the result TRS).
        rotation: Quat,
        /// World-space scale at drag start.
        start_scale: Vec3,
    },
}

impl GizmoDrag {
    /// Snapshot a drag start. `world_trs` is the entity's world-space TRS at
    /// this moment; `center` is the gizmo origin in screen space.
    pub fn begin(
        mode: GizmoMode,
        handle: GizmoHandle,
        frame: &GizmoFrame,
        pointer_ray: Option<Ray3d>,
        pointer: egui::Pos2,
        center: egui::Pos2,
        world_trs: WorldTrs,
    ) -> Option<Self> {
        let (translation, rotation, scale) = world_trs;
        match (mode, handle) {
            (GizmoMode::Translate, GizmoHandle::Axis(axis)) => {
                let axis_dir = frame.axes[axis];
                let pointer_ray = pointer_ray?;
                let plane_normal = axis_drag_plane(axis_dir, *pointer_ray.direction)?;
                let point = ray_plane_point(pointer_ray, frame.origin, plane_normal)?;
                Some(Self::Translate {
                    axis,
                    axis_dir,
                    origin: frame.origin,
                    plane_normal,
                    start_offset: (point - frame.origin).dot(axis_dir),
                    start_translation: translation,
                    rotation,
                    scale,
                })
            }
            (GizmoMode::Rotate, GizmoHandle::Axis(axis)) => {
                let axis_dir = frame.axes[axis];
                let point = ray_plane_point(pointer_ray?, frame.origin, axis_dir)?;
                let start_vec = (point - frame.origin).try_normalize()?;
                Some(Self::Rotate {
                    axis,
                    axis_dir,
                    origin: frame.origin,
                    start_vec,
                    translation,
                    start_rotation: rotation,
                    scale,
                })
            }
            (GizmoMode::Scale, handle) => {
                let start_distance = pointer.distance(center).max(1.0);
                Some(Self::Scale {
                    handle,
                    start_distance,
                    translation,
                    rotation,
                    start_scale: scale,
                })
            }
            _ => None,
        }
    }

    /// The handle this drag is operating on (for highlight drawing).
    pub fn handle(&self) -> GizmoHandle {
        match *self {
            Self::Translate { axis, .. } | Self::Rotate { axis, .. } => GizmoHandle::Axis(axis),
            Self::Scale { handle, .. } => handle,
        }
    }

    /// Compute the dragged entity's new world-space TRS from the current
    /// pointer position. `None` when this frame's pointer ray is degenerate
    /// (parallel to the drag axis/plane) — the caller keeps the last value.
    pub fn apply(
        &self,
        pointer_ray: Option<Ray3d>,
        pointer: egui::Pos2,
        center: egui::Pos2,
    ) -> Option<WorldTrs> {
        match *self {
            Self::Translate {
                axis_dir,
                origin,
                plane_normal,
                start_offset,
                start_translation,
                rotation,
                scale,
                ..
            } => {
                let point = ray_plane_point(pointer_ray?, origin, plane_normal)?;
                let offset = (point - origin).dot(axis_dir);
                let translation = start_translation + axis_dir * (offset - start_offset);
                Some((translation, rotation, scale))
            }
            Self::Rotate {
                axis_dir,
                origin,
                start_vec,
                translation,
                start_rotation,
                scale,
                ..
            } => {
                let point = ray_plane_point(pointer_ray?, origin, axis_dir)?;
                let current = (point - origin).try_normalize()?;
                let angle = signed_angle(start_vec, current, axis_dir);
                let rotation = Quat::from_axis_angle(axis_dir, angle) * start_rotation;
                Some((translation, rotation, scale))
            }
            Self::Scale {
                handle,
                start_distance,
                translation,
                rotation,
                start_scale,
            } => {
                let ratio = (pointer.distance(center) / start_distance).max(MIN_SCALE);
                let mut scale = start_scale;
                match handle {
                    GizmoHandle::Axis(axis) => {
                        scale[axis] = (scale[axis].abs() * ratio)
                            .max(MIN_SCALE)
                            .copysign(scale[axis]);
                    }
                    GizmoHandle::Uniform => scale *= ratio,
                }
                Some((translation, rotation, scale))
            }
        }
    }
}

/// Convert a world-space TRS into the entity's local [`Transform`], given
/// the parent's world affine (`None` for hierarchy roots).
pub fn world_trs_to_local(world_trs: WorldTrs, parent: Option<Affine3A>) -> Transform {
    let (translation, rotation, scale) = world_trs;
    let world = Affine3A::from_scale_rotation_translation(scale, rotation, translation);
    let local = match parent {
        Some(parent) => parent.inverse() * world,
        None => world,
    };
    let (scale, rotation, translation) = local.to_scale_rotation_translation();
    Transform {
        translation,
        rotation,
        scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_camera::{view_matrix, Camera};
    use moonfield_math::GlobalTransform;

    const RECT: egui::Rect =
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 720.0));

    /// The demo scene's camera pose: at (0, 2.5, 6) looking at the origin.
    fn demo_view_proj() -> Mat4 {
        let transform = Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y);
        Camera::default().projection_matrix(1280.0 / 720.0)
            * view_matrix(&GlobalTransform::from(transform))
    }

    #[test]
    fn test_orbit_camera_round_trip() {
        let original = Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y);
        let camera = OrbitCamera::from_transform(&original);
        let rebuilt = camera.transform();
        assert!(rebuilt.translation.distance(original.translation) < 1e-4);
        // Same view direction: both look at their respective pivots.
        let forward = |t: &Transform| t.rotation * Vec3::NEG_Z;
        assert!(forward(&rebuilt).distance(forward(&original)) < 1e-4);
    }

    #[test]
    fn test_orbit_camera_pitch_clamped() {
        let mut camera = OrbitCamera::from_transform(&Transform::IDENTITY);
        camera.orbit(egui::vec2(0.0, -100_000.0));
        assert!(camera.pitch <= MAX_PITCH);
        camera.orbit(egui::vec2(0.0, 200_000.0));
        assert!(camera.pitch >= MIN_PITCH);
    }

    #[test]
    fn test_orbit_camera_zoom_clamped() {
        let mut camera = OrbitCamera::from_transform(&Transform::IDENTITY);
        camera.zoom(1e9);
        assert!(camera.distance >= MIN_DISTANCE);
        camera.zoom(-1e9);
        assert!(camera.distance <= MAX_DISTANCE);
    }

    #[test]
    fn test_orbit_camera_pan_moves_pivot_in_view_plane() {
        let mut camera = OrbitCamera::from_transform(&Transform::IDENTITY);
        let before = camera.pivot;
        camera.pan(egui::vec2(100.0, 0.0));
        // Identity camera: right is +X, so dragging right pans the pivot -X.
        assert!(camera.pivot.x < before.x);
        assert!((camera.pivot.z - before.z).abs() < 1e-6);
    }

    #[test]
    fn test_world_to_screen_projects_origin_at_center() {
        // The test camera looks straight at the origin: it projects to the
        // exact center of the rect.
        let screen = world_to_screen(Vec3::ZERO, demo_view_proj(), RECT).unwrap();
        assert!((screen.x - 640.0).abs() < 1.0);
        assert!((screen.y - 360.0).abs() < 1.0);
    }

    #[test]
    fn test_world_to_screen_behind_camera_is_none() {
        assert!(world_to_screen(Vec3::new(0.0, 2.5, 12.0), demo_view_proj(), RECT).is_none());
    }

    #[test]
    fn test_screen_ray_round_trip() {
        let view_proj = demo_view_proj();
        let point = Vec3::new(1.0, 0.5, -2.0);
        let screen = world_to_screen(point, view_proj, RECT).unwrap();
        let ray = screen_to_ray(screen, RECT, view_proj).unwrap();
        // The ray must pass through the original point: distance point↔ray ≈ 0.
        let to_point = point - ray.origin;
        let along = to_point.dot(*ray.direction);
        let closest = ray.at(along);
        assert!(closest.distance(point) < 1e-3, "closest={closest:?}");
    }

    #[test]
    fn test_axis_drag_plane_stable_when_axis_faces_camera() {
        // The Z axis at the origin points almost straight at the demo
        // camera — the geometry that made the old closest-point-between-lines
        // drag explode. The drag plane still exists (the axis is not exactly
        // parallel to the ray), and a small mouse drag must produce a small,
        // finite translation.
        let view_proj = demo_view_proj();
        let frame = GizmoFrame::new(Vec3::ZERO, Quat::IDENTITY);
        let center = world_to_screen(frame.origin, view_proj, RECT).unwrap();
        let grab = center + egui::vec2(4.0, -3.0);

        let drag = GizmoDrag::begin(
            GizmoMode::Translate,
            GizmoHandle::Axis(2),
            &frame,
            screen_to_ray(grab, RECT, view_proj),
            grab,
            center,
            (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
        )
        .unwrap();

        let target = grab + egui::vec2(30.0, 10.0);
        let ray = screen_to_ray(target, RECT, view_proj);
        let (translation, _, _) = drag.apply(ray, target, center).unwrap();
        assert!(translation.is_finite(), "translation={translation:?}");
        assert!(
            translation.length() < 5.0,
            "near-parallel axis drag exploded: {translation:?}"
        );
    }

    #[test]
    fn test_axis_drag_plane_none_when_axis_parallel_to_ray() {
        // Ray exactly along the axis: no drag plane exists.
        let ray = Ray3d::new(Vec3::new(0.0, 0.0, 5.0), Dir3::NEG_Z);
        assert!(axis_drag_plane(Vec3::NEG_Z, *ray.direction).is_none());
    }

    /// Gizmo handles are screen-space constructs: the axis segment must be
    /// exactly `GIZMO_SCREEN_LENGTH` points at any entity distance, for
    /// foreshortened axes too (regression: the handle used to grow/shrink
    /// with distance because its length was computed in world space).
    #[test]
    fn test_axis_segment_constant_screen_length() {
        let view_proj = demo_view_proj();
        for origin in [
            Vec3::ZERO,
            Vec3::new(0.0, 1.0, 3.0),
            Vec3::new(-2.0, 4.0, 1.0),
        ] {
            let frame = GizmoFrame::new(origin, Quat::IDENTITY);
            for axis in 0..3 {
                if let Some((start, end)) = frame.axis_segment(axis, view_proj, RECT) {
                    let length = (end - start).length();
                    assert!(
                        (length - GIZMO_SCREEN_LENGTH).abs() < 1e-3,
                        "origin={origin:?} axis={axis}: length {length}"
                    );
                }
                // A None means the axis points straight at the camera from
                // this origin — legitimate, no handle to draw.
            }
        }
    }

    /// The full write-back loop the viewport performs per drag frame:
    /// snapshot the world TRS (decomposed from the global affine), apply the
    /// drag, convert back to local. Rotation and scale must survive intact —
    /// regression test for rotation turning into ±inf while translating.
    #[test]
    fn test_translate_write_back_preserves_rotation_and_scale() {
        let view_proj = demo_view_proj();
        let cases: &[(WorldTrs, Option<Affine3A>)] = &[
            // Root entity, rotated and non-uniformly scaled.
            (
                (
                    Vec3::new(1.0, 2.0, 3.0),
                    Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
                    Vec3::new(0.5, 2.0, 1.0),
                ),
                None,
            ),
            // Child of a rotated, uniformly scaled, translated parent.
            (
                (
                    Vec3::new(0.25, 0.5, -1.0),
                    Quat::from_rotation_x(0.3),
                    Vec3::splat(0.5),
                ),
                Some(Affine3A::from_scale_rotation_translation(
                    Vec3::splat(2.0),
                    Quat::from_rotation_z(0.7),
                    Vec3::new(-0.75, 0.0, 0.0),
                )),
            ),
        ];

        for &((start_translation, start_rotation, start_scale), parent) in cases {
            // The drag snapshots the *decomposed global* TRS, exactly as the
            // viewport does from the entity's GlobalTransform. glam's
            // decompose returns (scale, rotation, translation); the gizmo
            // pipeline works in (translation, rotation, scale) order.
            let global = Affine3A::from_scale_rotation_translation(
                start_scale,
                start_rotation,
                start_translation,
            );
            let (s, r, t) = global.to_scale_rotation_translation();
            let world_trs = (t, r, s);
            let frame = GizmoFrame::new(world_trs.0, world_trs.1);
            let center = world_to_screen(world_trs.0, view_proj, RECT).unwrap();
            let (start, end) = frame.axis_segment(0, view_proj, RECT).unwrap();
            let grab = start + (end - start) * 0.5;

            let drag = GizmoDrag::begin(
                GizmoMode::Translate,
                GizmoHandle::Axis(0),
                &frame,
                screen_to_ray(grab, RECT, view_proj),
                grab,
                center,
                world_trs,
            )
            .expect("drag begins");

            // Simulate several drag frames: each applies and writes back the
            // local transform, mirroring the viewport's per-frame write.
            // The baseline local TRS is the write-back of the pre-drag
            // state; a translate drag must leave its rotation and scale
            // untouched (for a child entity these differ from the world TRS
            // by the parent's inverse, so compare against the baseline).
            let baseline = world_trs_to_local(world_trs, parent);
            let mut local = baseline;
            for step in 1..=5 {
                let pos = grab + (end - start) * 0.04 * step as f32;
                let trs = drag
                    .apply(screen_to_ray(pos, RECT, view_proj), pos, center)
                    .expect("apply");
                local = world_trs_to_local(trs, parent);
                assert!(
                    local.rotation.is_finite() && local.scale.is_finite(),
                    "step {step}: rotation={:?} scale={:?}",
                    local.rotation,
                    local.scale,
                );
                assert!(
                    (local.rotation.length() - 1.0).abs() < 1e-3,
                    "step {step}: non-unit rotation {:?}",
                    local.rotation
                );
            }
            assert!(
                local.rotation.dot(baseline.rotation).abs() > 0.999,
                "rotation drifted: {:?} vs {:?}",
                local.rotation,
                baseline.rotation
            );
            assert!(
                (local.scale - baseline.scale).length() < 1e-3,
                "scale drifted: {:?} vs {:?}",
                local.scale,
                baseline.scale
            );
        }
    }

    #[test]
    fn test_translate_drag_moves_along_axis() {
        let view_proj = demo_view_proj();
        let frame = GizmoFrame::new(Vec3::ZERO, Quat::IDENTITY);
        let (start, end) = frame.axis_segment(0, view_proj, RECT).unwrap();
        let center = world_to_screen(frame.origin, view_proj, RECT).unwrap();

        let begin_ray = screen_to_ray(start, RECT, view_proj);
        let drag = GizmoDrag::begin(
            GizmoMode::Translate,
            GizmoHandle::Axis(0),
            &frame,
            begin_ray,
            start,
            center,
            (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
        )
        .unwrap();

        // Drag toward the axis endpoint: translation gains +X, nothing else.
        let target = start + (end - start) * 0.5;
        let ray = screen_to_ray(target, RECT, view_proj);
        let (translation, _, _) = drag.apply(ray, target, center).unwrap();
        assert!(translation.x > 0.1, "translation={translation:?}");
        assert!(translation.y.abs() < 1e-4);
        assert!(translation.z.abs() < 1e-4);
    }

    #[test]
    fn test_rotate_drag_rotates_around_axis() {
        let view_proj = demo_view_proj();
        let frame = GizmoFrame::new(Vec3::ZERO, Quat::IDENTITY);
        let center = world_to_screen(frame.origin, view_proj, RECT).unwrap();
        let ring = frame.ring_points(1, view_proj, RECT);
        assert!(ring.len() > 24);

        // Grab at ring angle 0, drag to ring angle ~90° (polyline index 12
        // of 48).
        let grab = ring[0];
        let begin_ray = screen_to_ray(grab, RECT, view_proj);
        let drag = GizmoDrag::begin(
            GizmoMode::Rotate,
            GizmoHandle::Axis(1),
            &frame,
            begin_ray,
            grab,
            center,
            (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
        )
        .unwrap();

        let target = ring[12];
        let ray = screen_to_ray(target, RECT, view_proj);
        let (_, rotation, _) = drag.apply(ray, target, center).unwrap();
        let expected = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2);
        assert!(
            rotation.dot(expected).abs() > 0.999,
            "rotation={rotation:?}"
        );
    }

    #[test]
    fn test_scale_drag_uniform_and_axis() {
        let frame = GizmoFrame::new(Vec3::ZERO, Quat::IDENTITY);
        let center = egui::pos2(100.0, 100.0);
        let grab = egui::pos2(150.0, 100.0); // distance 50

        let uniform = GizmoDrag::begin(
            GizmoMode::Scale,
            GizmoHandle::Uniform,
            &frame,
            None,
            grab,
            center,
            (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
        )
        .unwrap();
        // Doubling the pointer distance doubles every axis.
        let (_, _, scale) = uniform
            .apply(None, egui::pos2(200.0, 100.0), center)
            .unwrap();
        assert!((scale - Vec3::splat(2.0)).length() < 1e-5);

        let axis = GizmoDrag::begin(
            GizmoMode::Scale,
            GizmoHandle::Axis(2),
            &frame,
            None,
            grab,
            center,
            (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
        )
        .unwrap();
        // Pointer at distance 25 → ratio 0.5 on Z only.
        let (_, _, scale) = axis.apply(None, egui::pos2(125.0, 100.0), center).unwrap();
        assert!((scale.z - 0.5).abs() < 1e-5);
        assert_eq!(scale.x, 1.0);
        assert_eq!(scale.y, 1.0);
    }

    #[test]
    fn test_hit_test_finds_nearest_axis() {
        let view_proj = demo_view_proj();
        let frame = GizmoFrame::new(Vec3::ZERO, Quat::IDENTITY);
        let (start, end) = frame.axis_segment(0, view_proj, RECT).unwrap();
        let on_x = start + (end - start) * 0.5;
        assert_eq!(
            hit_test(GizmoMode::Translate, &frame, view_proj, RECT, on_x),
            Some(GizmoHandle::Axis(0))
        );
        // Far away from every handle: no hit.
        assert_eq!(
            hit_test(
                GizmoMode::Translate,
                &frame,
                view_proj,
                RECT,
                egui::pos2(5.0, 5.0)
            ),
            None
        );
    }

    #[test]
    fn test_hit_test_scale_center() {
        let view_proj = demo_view_proj();
        let frame = GizmoFrame::new(Vec3::ZERO, Quat::IDENTITY);
        let center = world_to_screen(frame.origin, view_proj, RECT).unwrap();
        assert_eq!(
            hit_test(GizmoMode::Scale, &frame, view_proj, RECT, center),
            Some(GizmoHandle::Uniform)
        );
    }

    #[test]
    fn test_world_trs_to_local_with_parent() {
        let parent = Affine3A::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 0.0, 0.0),
        );
        let local = world_trs_to_local(
            (Vec3::new(12.0, 2.0, 0.0), Quat::IDENTITY, Vec3::ONE),
            Some(parent),
        );
        // World 12 = parent 10 + local 1 * scale 2 → local x = 1.
        assert!((local.translation - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-5);
        assert!((local.scale - Vec3::splat(0.5)).length() < 1e-5);
    }

    #[test]
    fn test_world_trs_to_local_root_passthrough() {
        let local = world_trs_to_local((Vec3::X, Quat::IDENTITY, Vec3::splat(3.0)), None);
        assert_eq!(local.translation, Vec3::X);
        assert_eq!(local.scale, Vec3::splat(3.0));
    }
}

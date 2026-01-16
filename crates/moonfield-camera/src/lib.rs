//! Camera module for the Moonfield rendering engine
//!
//! This module provides camera implementations for 3D graphics applications,
//! including perspective and orthographic cameras with various utility functions.

use moonfield_core::math::geometry::Ray;
use moonfield_core::math::*;

/// Type alias for a basic camera
pub type Camera = PerspectiveCamera;

/// Camera trait that defines the interface for all camera types
pub trait CameraTrait {
    /// Get the view matrix for this camera
    fn view_matrix(&self) -> Mat4;

    /// Get the projection matrix for this camera
    fn projection_matrix(&self) -> Mat4;

    /// Get the view-projection matrix for this camera
    fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Get the inverse view-projection matrix
    fn inverse_view_projection_matrix(&self) -> Mat4 {
        self.view_projection_matrix().try_inverse().unwrap_or(Mat4::identity())
    }

    /// Get the camera's position in world space
    fn position(&self) -> Point3;

    /// Get the camera's forward direction in world space
    fn forward(&self) -> Vec3;

    /// Get the camera's up direction in world space
    fn up(&self) -> Vec3;

    /// Get the camera's right direction in world space
    fn right(&self) -> Vec3;
}

/// Perspective camera implementation
#[derive(Debug, Clone)]
pub struct PerspectiveCamera {
    /// Camera position in world space
    pub position: Point3,
    /// Camera orientation as a quaternion
    pub orientation: Quat,
    /// Field of view in radians
    pub fov_y: f32,
    /// Aspect ratio (width / height)
    pub aspect_ratio: f32,
    /// Near clipping plane distance
    pub near_plane: f32,
    /// Far clipping plane distance
    pub far_plane: f32,
}

impl PerspectiveCamera {
    /// Create a new perspective camera
    pub fn new(
        position: Point3, orientation: Quat, fov_y: f32, aspect_ratio: f32,
        near_plane: f32, far_plane: f32,
    ) -> Self {
        Self {
            position,
            orientation,
            fov_y,
            aspect_ratio,
            near_plane,
            far_plane,
        }
    }

    /// Create a perspective camera with a look-at transformation
    pub fn look_at(
        eye: Point3, target: Point3, up: Vec3, fov_y: f32, aspect_ratio: f32,
        near_plane: f32, far_plane: f32,
    ) -> Self {
        let forward = (target - eye).normalize();
        let right = up.cross(&forward).normalize();
        let corrected_up = forward.cross(&right).normalize();

        // Create rotation matrix from basis vectors
        let rotation_matrix = nalgebra::Rotation3::from_matrix_unchecked(
            nalgebra::Matrix3::from_columns(&[right, corrected_up, forward]),
        );

        // Convert to quaternion
        let rotation = Quat::from_rotation_matrix(&rotation_matrix);

        Self::new(eye, rotation, fov_y, aspect_ratio, near_plane, far_plane)
    }

    /// Set the camera's look-at target
    pub fn set_look_at(&mut self, eye: Point3, target: Point3, up: Vec3) {
        let forward = (target - eye).normalize();
        let right = up.cross(&forward).normalize();
        let corrected_up = forward.cross(&right).normalize();

        // Create rotation matrix from basis vectors
        let rotation_matrix = nalgebra::Rotation3::from_matrix_unchecked(
            nalgebra::Matrix3::from_columns(&[right, corrected_up, forward]),
        );

        // Convert to quaternion
        let rotation = Quat::from_rotation_matrix(&rotation_matrix);

        self.position = eye;
        self.orientation = rotation;
    }

    /// Move the camera by an offset in world space
    pub fn translate(&mut self, offset: Vec3) {
        self.position += offset;
    }

    /// Move the camera by an offset in local space (relative to camera orientation)
    pub fn translate_local(&mut self, offset: Vec3) {
        let world_offset = self.orientation * offset;
        self.position += world_offset;
    }

    /// Rotate the camera
    pub fn rotate(&mut self, rotation: Quat) {
        self.orientation = rotation * self.orientation;
        self.orientation.renormalize();
    }

    /// Pitch the camera (rotate around right axis)
    pub fn pitch(&mut self, angle: f32) {
        let right = self.right();
        let pitch_rotation =
            Quat::from_axis_angle(&nalgebra::Unit::new_normalize(right), angle);
        self.rotate(pitch_rotation);
    }

    /// Yaw the camera (rotate around up axis)
    pub fn yaw(&mut self, angle: f32) {
        let world_up = Vec3::y_axis().into_inner(); // Use global up for yaw
        let yaw_rotation = Quat::from_axis_angle(
            &nalgebra::Unit::new_normalize(world_up),
            angle,
        );
        self.rotate(yaw_rotation);
    }

    /// Roll the camera (rotate around forward axis)
    pub fn roll(&mut self, angle: f32) {
        let forward = self.forward();
        let roll_rotation = Quat::from_axis_angle(
            &nalgebra::Unit::new_normalize(forward),
            angle,
        );
        self.rotate(roll_rotation);
    }

    /// Set the field of view
    pub fn set_fov(&mut self, fov_y: f32) {
        self.fov_y = fov_y.clamp(0.01, PI - 0.01); // Prevent invalid FOV values
    }

    /// Set the aspect ratio
    pub fn set_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.aspect_ratio = aspect_ratio.max(0.001); // Prevent division by zero
    }

    /// Set the clipping planes
    pub fn set_clipping_planes(&mut self, near: f32, far: f32) {
        self.near_plane = near.max(0.001); // Prevent negative/near-zero near plane
        self.far_plane = far.max(self.near_plane + 0.001); // Ensure far > near
    }

    /// Get the camera's forward direction in world space
    pub fn forward(&self) -> Vec3 {
        self.orientation.transform_vector(&(-Vec3::z_axis()))
    }

    /// Get the camera's up direction in world space
    pub fn up(&self) -> Vec3 {
        self.orientation.transform_vector(&Vec3::y_axis())
    }

    /// Get the camera's right direction in world space
    pub fn right(&self) -> Vec3 {
        self.orientation.transform_vector(&Vec3::x_axis())
    }

    /// Get the camera's view matrix
    pub fn view_matrix(&self) -> Mat4 {
        let view_pos = self.position;
        let view_target = self.position + self.forward();
        let view_up = self.up();
        Mat4::look_at_rh(&view_pos, &view_target, &view_up)
    }

    /// Get the camera's projection matrix
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::new_perspective(
            self.aspect_ratio,
            self.fov_y,
            self.near_plane,
            self.far_plane,
        )
    }

    /// Get the camera's view-projection matrix
    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Get the inverse view-projection matrix
    pub fn inverse_view_projection_matrix(&self) -> Mat4 {
        self.view_projection_matrix().try_inverse().unwrap_or(Mat4::identity())
    }

    /// Convert a point from world space to screen space
    pub fn world_to_screen(
        &self, world_point: Point3, viewport_width: f32, viewport_height: f32,
    ) -> Option<Point3> {
        let clip_space =
            self.view_projection_matrix() * world_point.to_homogeneous();

        if clip_space.w == 0.0 {
            return None;
        }

        let ndc = Point3::from(clip_space.xyz() / clip_space.w);

        // Convert from NDC to screen coordinates
        let screen_x = (ndc.x + 1.0) * 0.5 * viewport_width;
        let screen_y = (1.0 - ndc.y) * 0.5 * viewport_height; // Flip Y axis
        let screen_z = (ndc.z + 1.0) * 0.5; // Depth in [0, 1]

        Some(Point3::new(screen_x, screen_y, screen_z))
    }

    /// Convert a point from screen space to world space
    pub fn screen_to_world(
        &self, screen_point: Point3, viewport_width: f32, viewport_height: f32,
    ) -> Option<Ray> {
        // Convert screen coordinates to NDC
        let ndc_x = (screen_point.x / viewport_width) * 2.0 - 1.0;
        let ndc_y = (1.0 - (screen_point.y / viewport_height)) * 2.0 - 1.0;
        let _ndc_z = screen_point.z * 2.0 - 1.0; // Z coordinate is used for depth but not in this calculation

        // Create NDC points for near and far planes
        let near_point = Point3::new(ndc_x, ndc_y, -1.0);
        let far_point = Point3::new(ndc_x, ndc_y, 1.0);

        // Transform from clip space to world space
        let inv_vp = self.inverse_view_projection_matrix();

        let world_near =
            Point3::from_homogeneous(inv_vp * near_point.to_homogeneous())?;
        let world_far =
            Point3::from_homogeneous(inv_vp * far_point.to_homogeneous())?;

        // Create a ray from the near point in the direction toward the far point
        let direction = (world_far - world_near).normalize();
        Some(Ray::new(world_near, direction))
    }

    /// Get the frustum corners in world space
    pub fn frustum_corners(&self) -> [Point3; 8] {
        // Get the inverse view-projection matrix
        let inv_view_proj = self.inverse_view_projection_matrix();

        // Define the 8 corners of the canonical view volume (-1 to 1 in all dimensions)
        let corners_ndc = [
            Point3::new(-1.0, -1.0, -1.0), // Near bottom-left
            Point3::new(1.0, -1.0, -1.0),  // Near bottom-right
            Point3::new(1.0, 1.0, -1.0),   // Near top-right
            Point3::new(-1.0, 1.0, -1.0),  // Near top-left
            Point3::new(-1.0, -1.0, 1.0),  // Far bottom-left
            Point3::new(1.0, -1.0, 1.0),   // Far bottom-right
            Point3::new(1.0, 1.0, 1.0),    // Far top-right
            Point3::new(-1.0, 1.0, 1.0),   // Far top-left
        ];

        // Transform each corner to world space
        let mut corners_world = [Point3::origin(); 8];
        for (i, &corner) in corners_ndc.iter().enumerate() {
            let world_corner = Point3::from_homogeneous(
                inv_view_proj * corner.to_homogeneous(),
            )
            .expect("Failed to convert corner to world space");
            corners_world[i] = world_corner;
        }

        corners_world
    }
}

impl CameraTrait for PerspectiveCamera {
    fn view_matrix(&self) -> Mat4 {
        self.view_matrix()
    }

    fn projection_matrix(&self) -> Mat4 {
        self.projection_matrix()
    }

    fn position(&self) -> Point3 {
        self.position
    }

    fn forward(&self) -> Vec3 {
        self.forward()
    }

    fn up(&self) -> Vec3 {
        self.up()
    }

    fn right(&self) -> Vec3 {
        self.right()
    }
}

/// Orthographic camera implementation
#[derive(Debug, Clone)]
pub struct OrthographicCamera {
    /// Camera position in world space
    pub position: Point3,
    /// Camera orientation as a quaternion
    pub orientation: Quat,
    /// Left clipping plane
    pub left: f32,
    /// Right clipping plane
    pub right: f32,
    /// Bottom clipping plane
    pub bottom: f32,
    /// Top clipping plane
    pub top: f32,
    /// Near clipping plane
    pub near_plane: f32,
    /// Far clipping plane
    pub far_plane: f32,
}

impl OrthographicCamera {
    /// Create a new orthographic camera
    pub fn new(
        position: Point3, orientation: Quat, left: f32, right: f32,
        bottom: f32, top: f32, near_plane: f32, far_plane: f32,
    ) -> Self {
        Self {
            position,
            orientation,
            left,
            right,
            bottom,
            top,
            near_plane,
            far_plane,
        }
    }

    /// Create an orthographic camera with a look-at transformation
    pub fn look_at(
        eye: Point3, target: Point3, up: Vec3, left: f32, right: f32,
        bottom: f32, top: f32, near_plane: f32, far_plane: f32,
    ) -> Self {
        let forward = (target - eye).normalize();
        let right_vec = up.cross(&forward).normalize();
        let corrected_up = forward.cross(&right_vec).normalize();

        // Create rotation matrix from basis vectors
        let rotation_matrix = nalgebra::Rotation3::from_matrix_unchecked(
            nalgebra::Matrix3::from_columns(&[
                right_vec,
                corrected_up,
                forward,
            ]),
        );

        // Convert to quaternion
        let rotation = Quat::from_rotation_matrix(&rotation_matrix);

        Self::new(
            eye, rotation, left, right, bottom, top, near_plane, far_plane,
        )
    }

    /// Set the camera's look-at target
    pub fn set_look_at(&mut self, eye: Point3, target: Point3, up: Vec3) {
        let forward = (target - eye).normalize();
        let right_vec = up.cross(&forward).normalize();
        let corrected_up = forward.cross(&right_vec).normalize();

        // Create rotation matrix from basis vectors
        let rotation_matrix = nalgebra::Rotation3::from_matrix_unchecked(
            nalgebra::Matrix3::from_columns(&[
                right_vec,
                corrected_up,
                forward,
            ]),
        );

        // Convert to quaternion
        let rotation = Quat::from_rotation_matrix(&rotation_matrix);

        self.position = eye;
        self.orientation = rotation;
    }

    /// Set the orthographic projection parameters
    pub fn set_projection(
        &mut self, left: f32, right: f32, bottom: f32, top: f32, near: f32,
        far: f32,
    ) {
        self.left = left;
        self.right = right;
        self.bottom = bottom;
        self.top = top;
        self.near_plane = near;
        self.far_plane = far;
    }

    /// Get the camera's forward direction in world space
    pub fn forward(&self) -> Vec3 {
        self.orientation.transform_vector(&(-Vec3::z_axis()))
    }

    /// Get the camera's up direction in world space
    pub fn up(&self) -> Vec3 {
        self.orientation.transform_vector(&Vec3::y_axis())
    }

    /// Get the camera's right direction in world space
    pub fn right(&self) -> Vec3 {
        self.orientation.transform_vector(&Vec3::x_axis())
    }

    /// Get the camera's view matrix
    pub fn view_matrix(&self) -> Mat4 {
        let view_pos = self.position;
        let view_target = self.position + self.forward();
        let view_up = self.up();
        Mat4::look_at_rh(&view_pos, &view_target, &view_up)
    }

    /// Get the camera's projection matrix
    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::new_orthographic(
            self.left,
            self.right,
            self.bottom,
            self.top,
            self.near_plane,
            self.far_plane,
        )
    }
}

impl CameraTrait for OrthographicCamera {
    fn view_matrix(&self) -> Mat4 {
        self.view_matrix()
    }

    fn projection_matrix(&self) -> Mat4 {
        self.projection_matrix()
    }

    fn position(&self) -> Point3 {
        self.position
    }

    fn forward(&self) -> Vec3 {
        self.forward()
    }

    fn up(&self) -> Vec3 {
        self.up()
    }

    fn right(&self) -> Vec3 {
        self.right()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perspective_camera_creation() {
        let pos = Point3::new(0.0, 0.0, 5.0);
        let rot = Quat::identity();
        let camera =
            PerspectiveCamera::new(pos, rot, PI / 3.0, 16.0 / 9.0, 0.1, 100.0);

        assert_eq!(camera.position, pos);
        assert_eq!(camera.orientation, rot);
    }

    #[test]
    fn test_perspective_camera_look_at() {
        let eye = Point3::new(0.0, 0.0, 5.0);
        let target = Point3::new(0.0, 0.0, 0.0);
        let up = Vec3::y_axis().into_inner();

        let camera = PerspectiveCamera::look_at(
            eye,
            target,
            up,
            PI / 3.0,
            16.0 / 9.0,
            0.1,
            100.0,
        );

        assert_eq!(camera.position, eye);
    }

    #[test]
    fn test_perspective_camera_movement() {
        let mut camera = PerspectiveCamera::new(
            Point3::origin(),
            Quat::identity(),
            PI / 3.0,
            16.0 / 9.0,
            0.1,
            100.0,
        );

        let initial_pos = camera.position;
        camera.translate(Vec3::new(1.0, 0.0, 0.0));
        assert_ne!(camera.position, initial_pos);
    }

    #[test]
    fn test_perspective_camera_directions() {
        let camera = PerspectiveCamera::new(
            Point3::origin(),
            Quat::identity(),
            PI / 3.0,
            16.0 / 9.0,
            0.1,
            100.0,
        );

        let forward = camera.forward();
        let expected_forward = Vec3::new(0.0, 0.0, -1.0);
        assert!((forward - expected_forward).magnitude() < 1e-5);

        let up = camera.up();
        let expected_up = Vec3::y_axis().into_inner();
        assert!((up - expected_up).magnitude() < 1e-5);

        let right = camera.right();
        let expected_right = Vec3::x_axis().into_inner();
        assert!((right - expected_right).magnitude() < 1e-5);
    }

    #[test]
    fn test_orthographic_camera_creation() {
        let pos = Point3::new(0.0, 0.0, 5.0);
        let rot = Quat::identity();
        let camera = OrthographicCamera::new(
            pos, rot, -10.0, 10.0, -5.0, 5.0, 0.1, 100.0,
        );

        assert_eq!(camera.position, pos);
        assert_eq!(camera.orientation, rot);
    }
}

//! Math utilities for the Moonfield rendering engine
//!
//! This module provides convenient type aliases and utility functions
//! for common mathematical operations in 3D graphics and rendering.

pub use nalgebra;

// Common vector types
pub type Vec2 = nalgebra::Vector2<f32>;
pub type Vec3 = nalgebra::Vector3<f32>;
pub type Vec4 = nalgebra::Vector4<f32>;

pub type Vec2d = nalgebra::Vector2<f64>;
pub type Vec3d = nalgebra::Vector3<f64>;
pub type Vec4d = nalgebra::Vector4<f64>;

pub type Vec2i = nalgebra::Vector2<i32>;
pub type Vec3i = nalgebra::Vector3<i32>;
pub type Vec4i = nalgebra::Vector4<i32>;

pub type Vec2u = nalgebra::Vector2<u32>;
pub type Vec3u = nalgebra::Vector3<u32>;
pub type Vec4u = nalgebra::Vector4<u32>;

// Matrix types
pub type Mat2 = nalgebra::Matrix2<f32>;
pub type Mat3 = nalgebra::Matrix3<f32>;
pub type Mat4 = nalgebra::Matrix4<f32>;

pub type Mat2d = nalgebra::Matrix2<f64>;
pub type Mat3d = nalgebra::Matrix3<f64>;
pub type Mat4d = nalgebra::Matrix4<f64>;

// Point types
pub type Point2 = nalgebra::Point2<f32>;
pub type Point3 = nalgebra::Point3<f32>;

pub type Point2d = nalgebra::Point2<f64>;
pub type Point3d = nalgebra::Point3<f64>;

// Quaternion types
pub type Quat = nalgebra::UnitQuaternion<f32>;
pub type Quatd = nalgebra::UnitQuaternion<f64>;

// Transformation types
pub type Transform2 = nalgebra::Isometry2<f32>;
pub type Transform3 = nalgebra::Isometry3<f32>;

pub type Transform2d = nalgebra::Isometry2<f64>;
pub type Transform3d = nalgebra::Isometry3<f64>;

// Perspective and orthographic projection types
pub type Perspective3 = nalgebra::Perspective3<f32>;
pub type Orthographic3 = nalgebra::Orthographic3<f32>;

// Common constants
pub const PI: f32 = std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;
pub const E: f32 = std::f32::consts::E;

/// Utility functions for common mathematical operations
pub mod utils {
    use super::*;

    /// Convert degrees to radians
    #[inline]
    pub fn deg_to_rad(degrees: f32) -> f32 {
        degrees * PI / 180.0
    }

    /// Convert radians to degrees
    #[inline]
    pub fn rad_to_deg(radians: f32) -> f32 {
        radians * 180.0 / PI
    }

    /// Linear interpolation between two values
    #[inline]
    pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }

    /// Clamp a value between min and max
    #[inline]
    pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
        value.max(min).min(max)
    }

    /// Smoothstep interpolation
    #[inline]
    pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
        let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Create a look-at matrix for a camera
    pub fn look_at(eye: &Point3, target: &Point3, up: &Vec3) -> Mat4 {
        Mat4::look_at_rh(eye, target, up)
    }

    /// Create a perspective projection matrix
    pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        Mat4::new_perspective(aspect, fovy, near, far)
    }

    /// Create an orthographic projection matrix
    pub fn orthographic(
        left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32,
    ) -> Mat4 {
        Mat4::new_orthographic(left, right, bottom, top, near, far)
    }
}

/// Color utilities and types
pub mod color {
    use super::*;

    /// RGBA color with f32 components (0.0 to 1.0)
    pub type Color = Vec4;

    /// RGB color with f32 components (0.0 to 1.0)
    pub type Color3 = Vec3;

    /// Common color constants
    pub const WHITE: Color = Vec4::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Color = Vec4::new(0.0, 0.0, 0.0, 1.0);
    pub const RED: Color = Vec4::new(1.0, 0.0, 0.0, 1.0);
    pub const GREEN: Color = Vec4::new(0.0, 1.0, 0.0, 1.0);
    pub const BLUE: Color = Vec4::new(0.0, 0.0, 1.0, 1.0);
    pub const YELLOW: Color = Vec4::new(1.0, 1.0, 0.0, 1.0);
    pub const MAGENTA: Color = Vec4::new(1.0, 0.0, 1.0, 1.0);
    pub const CYAN: Color = Vec4::new(0.0, 1.0, 1.0, 1.0);
    pub const TRANSPARENT: Color = Vec4::new(0.0, 0.0, 0.0, 0.0);

    /// Create a color from RGB values (0-255)
    pub fn rgb(r: u8, g: u8, b: u8) -> Color {
        Vec4::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }

    /// Create a color from RGBA values (0-255)
    pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Vec4::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// Create a color from HSV values
    pub fn hsv(h: f32, s: f32, v: f32) -> Color3 {
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;

        let (r, g, b) = if h < 60.0 {
            (c, x, 0.0)
        } else if h < 120.0 {
            (x, c, 0.0)
        } else if h < 180.0 {
            (0.0, c, x)
        } else if h < 240.0 {
            (0.0, x, c)
        } else if h < 300.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        Vec3::new(r + m, g + m, b + m)
    }
}

/// Geometry utilities
pub mod geometry {
    use super::*;

    /// Represents a ray in 3D space
    #[derive(Debug, Clone, Copy)]
    pub struct Ray {
        pub origin: Point3,
        pub direction: Vec3,
    }

    impl Ray {
        /// Create a new ray
        pub fn new(origin: Point3, direction: Vec3) -> Self {
            Self { origin, direction: direction.normalize() }
        }

        /// Get a point along the ray at parameter t
        pub fn at(&self, t: f32) -> Point3 {
            self.origin + self.direction * t
        }
    }

    /// Represents a plane in 3D space
    #[derive(Debug, Clone, Copy)]
    pub struct Plane {
        pub normal: Vec3,
        pub distance: f32,
    }

    impl Plane {
        /// Create a new plane from a normal and distance from origin
        pub fn new(normal: Vec3, distance: f32) -> Self {
            Self { normal: normal.normalize(), distance }
        }

        /// Create a plane from three points
        pub fn from_points(p1: Point3, p2: Point3, p3: Point3) -> Self {
            let v1 = p2 - p1;
            let v2 = p3 - p1;
            let normal = v1.cross(&v2).normalize();
            let distance = normal.dot(&p1.coords);
            Self { normal, distance }
        }

        /// Calculate the distance from a point to this plane
        pub fn distance_to_point(&self, point: Point3) -> f32 {
            self.normal.dot(&point.coords) - self.distance
        }
    }

    /// Represents an axis-aligned bounding box
    #[derive(Debug, Clone, Copy)]
    pub struct AABB {
        pub min: Point3,
        pub max: Point3,
    }

    impl AABB {
        /// Create a new AABB
        pub fn new(min: Point3, max: Point3) -> Self {
            Self { min, max }
        }

        /// Get the center of the AABB
        pub fn center(&self) -> Point3 {
            Point3::from((self.min.coords + self.max.coords) * 0.5)
        }

        /// Get the size of the AABB
        pub fn size(&self) -> Vec3 {
            self.max - self.min
        }

        /// Check if a point is inside the AABB
        pub fn contains_point(&self, point: Point3) -> bool {
            point.x >= self.min.x
                && point.x <= self.max.x
                && point.y >= self.min.y
                && point.y <= self.max.y
                && point.z >= self.min.z
                && point.z <= self.max.z
        }

        /// Expand the AABB to include a point
        pub fn expand_to_include(&mut self, point: Point3) {
            self.min.x = self.min.x.min(point.x);
            self.min.y = self.min.y.min(point.y);
            self.min.z = self.min.z.min(point.z);
            self.max.x = self.max.x.max(point.x);
            self.max.y = self.max.y.max(point.y);
            self.max.z = self.max.z.max(point.z);
        }
    }
}

// Note: Transform is now defined in this crate rather than moonfield-transform
// We'll define it here if needed, or re-export from moonfield-transform if preferred
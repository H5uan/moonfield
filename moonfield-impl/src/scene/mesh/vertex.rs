use moonfield_core::math::{Vec2, Vec3, Vec4};

pub struct SimpleVertex {
    pub position: Vec3,
}

impl SimpleVertex {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { position: Vec3::new(x, y, z) }
    }
}

pub struct StaticVertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub tex_coord: Vec2,
    pub tangent: Vec4,
}

impl StaticVertex {
    /// Creates new vertex from given position and texture coordinates.
    pub fn from_pos_uv(position: Vec3, tex_coord: Vec2) -> Self {
        Self {
            position,
            tex_coord,
            normal: Vec3::new(0.0, 1.0, 0.0),
            tangent: Vec4::default(),
        }
    }

    /// Creates new vertex from given position and texture coordinates.
    pub fn from_pos_uv_normal(
        position: Vec3, tex_coord: Vec2, normal: Vec3,
    ) -> Self {
        Self { position, tex_coord, normal, tangent: Vec4::default() }
    }
}

//! Gaussian scene representation.

/// Number of spherical-harmonics coefficients per channel beyond the DC term
/// (degree-3 SH, 16 coefficients total minus 1 DC), times RGB.
pub const SH_REST_LEN: usize = 45;

/// A 3D Gaussian scene in structure-of-arrays (SoA) layout.
///
/// SoA keeps every attribute in one contiguous `Vec`, which maps 1:1 onto
/// GPU storage buffers (no interleaving stride games) and lets the rasterizer
/// / trainer upload or sort individual attributes independently.
///
/// Field correspondence with a standard 3DGS `.ply` file (see [`crate::splat::io::ply`]):
///
/// | Field       | PLY properties                | Notes                        |
/// |-------------|-------------------------------|------------------------------|
/// | `positions` | `x, y, z`                     | world-space mean             |
/// | `scales`    | `scale_0..2`                  | log-space, `exp` before use  |
/// | `rotations` | `rot_0..3`                    | quaternion (w, x, y, z)      |
/// | `opacities` | `opacity`                     | logit-space, sigmoid before use |
/// | `sh_dc`     | `f_dc_0..2`                   | degree-0 SH, RGB             |
/// | `sh_rest`   | `f_rest_0..44`                | 15 higher-order SH coeffs x RGB |
///
/// The PLY `nx, ny, nz` normal properties are parsed past and dropped —
/// 3DGS does not use them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GaussianScene {
    /// World-space Gaussian centers, one `[x, y, z]` per Gaussian.
    pub positions: Vec<[f32; 3]>,
    /// Log-space per-axis scales, one `[sx, sy, sz]` per Gaussian.
    pub scales: Vec<[f32; 3]>,
    /// Rotation quaternions `(w, x, y, z)` per Gaussian.
    pub rotations: Vec<[f32; 4]>,
    /// Logit-space opacity per Gaussian.
    pub opacities: Vec<f32>,
    /// Degree-0 (DC) spherical-harmonics color, RGB per Gaussian.
    pub sh_dc: Vec<[f32; 3]>,
    /// Remaining 15 SH coefficients x RGB per Gaussian (degree <= 3).
    pub sh_rest: Vec<[f32; SH_REST_LEN]>,
}

impl GaussianScene {
    /// Number of Gaussians in the scene.
    #[must_use]
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// Whether the scene contains no Gaussians.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

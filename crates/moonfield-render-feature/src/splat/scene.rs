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
/// Field correspondence with a `KHR_gaussian_splatting` glTF primitive (see
/// [`crate::splat::io::gltf`], which performs the conversions noted below):
///
/// | Field       | glTF attribute semantic                     | Notes                        |
/// |-------------|---------------------------------------------|------------------------------|
/// | `positions` | `POSITION`                                  | world-space mean             |
/// | `scales`    | `KHR_gaussian_splatting:SCALE`              | glTF is linear; stored here log-space, `exp` before use |
/// | `rotations` | `KHR_gaussian_splatting:ROTATION`           | glTF (x, y, z, w); stored here (w, x, y, z) |
/// | `opacities` | `KHR_gaussian_splatting:OPACITY`            | glTF is 0..=1; stored here logit-space, sigmoid before use |
/// | `sh_dc`     | `KHR_gaussian_splatting:SH_DEGREE_0_COEF_0` | degree-0 SH, RGB (verbatim)  |
/// | `sh_rest`   | `KHR_gaussian_splatting:SH_DEGREE_l_COEF_n` | 15 higher-order SH coeffs x RGB |
///
/// `sh_rest` is channel-blocked (3DGS `f_rest` order): coefficients 0..14 of
/// the red channel, then green, then blue. The glTF attributes hold one RGB
/// VEC3 per coefficient, so loading transposes; missing degrees zero-fill.
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

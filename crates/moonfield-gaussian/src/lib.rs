//! The CPU-side 3D Gaussian scene: the domain data and conventions shared by
//! the renderer (`moonfield-render-feature`'s splat modules) and the trainer
//! (`moonfield-ml`), with no framework dependencies.
//!
//! [`GaussianScene`] is the single owner of the attribute conventions; the
//! glTF loader (`moonfield-render-feature::splat::io`), the forward kernels,
//! and the trainer all speak them.

/// Number of spherical-harmonics coefficients per channel beyond the DC term
/// (degree-3 SH, 16 coefficients total minus 1 DC), times RGB.
pub const SH_REST_LEN: usize = 45;

/// Errors produced by [`GaussianScene`] construction and updates.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GaussianSceneError {
    /// An attribute array's length differs from the Gaussian count (the
    /// position count).
    #[error(
        "Gaussian attribute `{attribute}` has {actual} entries; expected {expected} \
         (one per position)"
    )]
    LengthMismatch {
        /// The mismatched attribute's name.
        attribute: &'static str,
        /// The expected length (the position count).
        expected: usize,
        /// The actual length.
        actual: usize,
    },
    /// An attribute value is NaN or infinite.
    #[error("Gaussian attribute `{attribute}` entry {index} is not finite")]
    NonFinite {
        /// The offending attribute's name.
        attribute: &'static str,
        /// The Gaussian index carrying the non-finite value.
        index: usize,
    },
}

/// A 3D Gaussian scene in structure-of-arrays (SoA) layout.
///
/// SoA keeps every attribute in one contiguous `Vec`, which maps 1:1 onto
/// GPU storage buffers (no interleaving stride games) and lets the rasterizer
/// / trainer upload or sort individual attributes independently.
///
/// Field correspondence with a `KHR_gaussian_splatting` glTF primitive (the
/// loader in `moonfield-render-feature::splat::io` performs the conversions
/// noted below):
///
/// | Attribute   | glTF attribute semantic                     | Notes                        |
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
///
/// The fields are private so every construction and update passes
/// validation: all attribute arrays hold one entry per Gaussian, and every
/// value is finite. (The space conventions — log-space scales, logit
/// opacities, (w, x, y, z) rotations — are caller-upheld; they cannot be
/// checked numerically.)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GaussianScene {
    /// World-space Gaussian centers, one `[x, y, z]` per Gaussian.
    positions: Vec<[f32; 3]>,
    /// Log-space per-axis scales, one `[sx, sy, sz]` per Gaussian.
    scales: Vec<[f32; 3]>,
    /// Rotation quaternions `(w, x, y, z)` per Gaussian.
    rotations: Vec<[f32; 4]>,
    /// Logit-space opacity per Gaussian.
    opacities: Vec<f32>,
    /// Degree-0 (DC) spherical-harmonics color, RGB per Gaussian.
    sh_dc: Vec<[f32; 3]>,
    /// Remaining 15 SH coefficients x RGB per Gaussian (degree <= 3).
    sh_rest: Vec<[f32; SH_REST_LEN]>,
}

impl GaussianScene {
    /// A scene from full attribute arrays, validated: every array holds one
    /// entry per position, and every value is finite.
    pub fn new(
        positions: Vec<[f32; 3]>,
        scales: Vec<[f32; 3]>,
        rotations: Vec<[f32; 4]>,
        opacities: Vec<f32>,
        sh_dc: Vec<[f32; 3]>,
        sh_rest: Vec<[f32; SH_REST_LEN]>,
    ) -> Result<Self, GaussianSceneError> {
        let scene = Self {
            positions,
            scales,
            rotations,
            opacities,
            sh_dc,
            sh_rest,
        };
        scene.validate()?;
        Ok(scene)
    }

    /// Append one Gaussian, validating its values (finite). Returns the new
    /// Gaussian's index.
    pub fn push(
        &mut self,
        position: [f32; 3],
        scale: [f32; 3],
        rotation: [f32; 4],
        opacity: f32,
        sh_dc: [f32; 3],
        sh_rest: [f32; SH_REST_LEN],
    ) -> Result<usize, GaussianSceneError> {
        let index = self.len();
        check_finite("positions", index, &position)?;
        check_finite("scales", index, &scale)?;
        check_finite("rotations", index, &rotation)?;
        check_finite("opacities", index, &[opacity])?;
        check_finite("sh_dc", index, &sh_dc)?;
        check_finite("sh_rest", index, &sh_rest)?;
        self.positions.push(position);
        self.scales.push(scale);
        self.rotations.push(rotation);
        self.opacities.push(opacity);
        self.sh_dc.push(sh_dc);
        self.sh_rest.push(sh_rest);
        Ok(index)
    }

    /// Check the scene's invariants: one entry per attribute per Gaussian,
    /// all values finite.
    pub fn validate(&self) -> Result<(), GaussianSceneError> {
        let count = self.len();
        for (attribute, actual) in [
            ("scales", self.scales.len()),
            ("rotations", self.rotations.len()),
            ("opacities", self.opacities.len()),
            ("sh_dc", self.sh_dc.len()),
            ("sh_rest", self.sh_rest.len()),
        ] {
            if actual != count {
                return Err(GaussianSceneError::LengthMismatch {
                    attribute,
                    expected: count,
                    actual,
                });
            }
        }
        for (index, position) in self.positions.iter().enumerate() {
            check_finite("positions", index, position)?;
        }
        for (index, scale) in self.scales.iter().enumerate() {
            check_finite("scales", index, scale)?;
        }
        for (index, rotation) in self.rotations.iter().enumerate() {
            check_finite("rotations", index, rotation)?;
        }
        for (index, opacity) in self.opacities.iter().enumerate() {
            check_finite("opacities", index, &[*opacity])?;
        }
        for (index, sh_dc) in self.sh_dc.iter().enumerate() {
            check_finite("sh_dc", index, sh_dc)?;
        }
        for (index, sh_rest) in self.sh_rest.iter().enumerate() {
            check_finite("sh_rest", index, sh_rest)?;
        }
        Ok(())
    }

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

    /// World-space centers, one per Gaussian.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// Log-space per-axis scales, one per Gaussian.
    pub fn scales(&self) -> &[[f32; 3]] {
        &self.scales
    }

    /// Rotation quaternions `(w, x, y, z)`, one per Gaussian.
    pub fn rotations(&self) -> &[[f32; 4]] {
        &self.rotations
    }

    /// Logit-space opacities, one per Gaussian.
    pub fn opacities(&self) -> &[f32] {
        &self.opacities
    }

    /// Degree-0 SH color, RGB per Gaussian.
    pub fn sh_dc(&self) -> &[[f32; 3]] {
        &self.sh_dc
    }

    /// Higher-order SH coefficients (channel-blocked), one per Gaussian.
    pub fn sh_rest(&self) -> &[[f32; SH_REST_LEN]] {
        &self.sh_rest
    }
}

fn check_finite(
    attribute: &'static str,
    index: usize,
    values: &[f32],
) -> Result<(), GaussianSceneError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(GaussianSceneError::NonFinite { attribute, index })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_accepts_matching_finite_attributes() {
        let scene = GaussianScene::new(
            vec![[1.0, 2.0, 3.0]],
            vec![[0.0; 3]],
            vec![[1.0, 0.0, 0.0, 0.0]],
            vec![0.0],
            vec![[0.1, 0.2, 0.3]],
            vec![[0.0; SH_REST_LEN]],
        )
        .expect("valid scene");
        assert_eq!(scene.len(), 1);
        assert_eq!(scene.positions()[0], [1.0, 2.0, 3.0]);
        assert_eq!(scene.sh_rest()[0], [0.0; SH_REST_LEN]);
    }

    #[test]
    fn test_new_rejects_mismatched_attribute_lengths() {
        let err = GaussianScene::new(
            vec![[1.0, 2.0, 3.0]],
            vec![[0.0; 3]],
            vec![[1.0, 0.0, 0.0, 0.0]],
            vec![0.0],
            vec![[0.1, 0.2, 0.3]],
            Vec::new(),
        )
        .unwrap_err();
        assert_eq!(
            err,
            GaussianSceneError::LengthMismatch {
                attribute: "sh_rest",
                expected: 1,
                actual: 0,
            }
        );
    }

    #[test]
    fn test_new_rejects_non_finite_values() {
        let err = GaussianScene::new(
            vec![[1.0, 2.0, 3.0]],
            vec![[0.0; 3]],
            vec![[1.0, 0.0, 0.0, 0.0]],
            vec![f32::NEG_INFINITY],
            vec![[0.1, 0.2, 0.3]],
            vec![[0.0; SH_REST_LEN]],
        )
        .unwrap_err();
        assert_eq!(
            err,
            GaussianSceneError::NonFinite {
                attribute: "opacities",
                index: 0,
            }
        );
    }

    #[test]
    fn test_push_appends_and_validates() {
        let mut scene = GaussianScene::default();
        let index = scene
            .push(
                [1.0, 2.0, 3.0],
                [0.0; 3],
                [1.0, 0.0, 0.0, 0.0],
                0.0,
                [0.1, 0.2, 0.3],
                [0.0; SH_REST_LEN],
            )
            .unwrap();
        assert_eq!(index, 0);
        assert_eq!(scene.len(), 1);
        assert!(
            scene
                .push(
                    [1.0, 2.0, 3.0],
                    [0.0; 3],
                    [1.0, 0.0, 0.0, 0.0],
                    f32::NAN,
                    [0.1, 0.2, 0.3],
                    [0.0; SH_REST_LEN],
                )
                .is_err()
        );
        // The rejected push changed nothing.
        assert_eq!(scene.len(), 1);
    }
}

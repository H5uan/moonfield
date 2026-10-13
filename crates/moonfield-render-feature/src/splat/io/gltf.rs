//! `KHR_gaussian_splatting` glTF loader for 3D Gaussian scenes.
//!
//! Parses the Khronos `KHR_gaussian_splatting` extension (Release Candidate):
//! a mesh primitive in POINTS mode whose `extensions` object carries
//! `KHR_gaussian_splatting`, with the splat data in standard attribute
//! semantics (`POSITION`, `KHR_gaussian_splatting:ROTATION`, `:SCALE`,
//! `:OPACITY`, `:SH_DEGREE_0_COEF_0`, `:SH_DEGREE_l_COEF_n`).
//!
//! Only **float** attributes are supported — the normalized int8/16 quantized
//! variants the extension allows are rejected, as are nested compression
//! sub-extensions (SPZ-style) and kernels other than `"ellipse"`.
//!
//! ## Render space → training space
//!
//! [`GaussianScene`] keeps the 3DGS training conventions (see
//! [`crate::splat::scene`]); the loader converts the glTF render-space values:
//!
//! - `SCALE` (linear, non-negative) → `scales = ln(scale)` component-wise;
//! - `OPACITY` (`0..=1`) → `opacities = logit(p) = ln(p / (1 - p))`;
//! - `ROTATION` (glTF quaternion order `(x, y, z, w)`) → `(w, x, y, z)`;
//! - `SH_DEGREE_0_COEF_0` → `sh_dc` verbatim — both formats store raw SH
//!   coefficients (the `0.282095 * c + 0.5` shading bias is applied at render
//!   time, never stored);
//! - `SH_DEGREE_l_COEF_n` (one RGB VEC3 per coefficient) → `sh_rest`
//!   channel-blocked layout: coefficient `c = l*l - 1 + n` of channel `ch`
//!   lands at `sh_rest[ch * 15 + c]`; missing degrees are zero-filled.
//!
//! Two implementation notes:
//!
//! - The document is parsed **without validation**: gltf-json maps the
//!   unknown `KHR_gaussian_splatting:*` attribute semantics to
//!   `Checked::Invalid`, which full validation rejects (and which collapses
//!   the typed attribute map to a single entry). The attribute map and the
//!   extension object are therefore read from the raw JSON, while accessor
//!   offsets come from the typed `gltf` document.
//! - Accessors are decoded manually from the buffer blobs (float component
//!   type only, `byteStride` honored).

use std::path::Path;

use serde_json::Value;

use crate::splat::scene::{GaussianScene, SH_REST_LEN};

/// The extension name as it appears in `extensions` and attribute semantics.
const EXTENSION: &str = "KHR_gaussian_splatting";

/// Errors returned by [`load_splat_gltf`].
#[derive(Debug, thiserror::Error)]
pub enum SplatGltfError {
    /// The glTF container or its buffers could not be read.
    #[error("failed to read the glTF container: {0}")]
    Gltf(#[from] gltf::Error),
    /// The glTF JSON document could not be parsed.
    #[error("failed to parse the glTF JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// No mesh primitive carries the `KHR_gaussian_splatting` extension.
    #[error("no mesh primitive carries the `KHR_gaussian_splatting` extension")]
    NoSplatPrimitive,
    /// Only the `"ellipse"` kernel is supported.
    #[error("unsupported splat kernel `{0}` (expected `ellipse`)")]
    UnsupportedKernel(String),
    /// Nested compression sub-extensions (e.g. SPZ) are not supported.
    #[error("unsupported `KHR_gaussian_splatting` sub-extension `{0}`")]
    UnsupportedSubExtension(String),
    /// A splat primitive must be in POINTS mode (0).
    #[error("splat primitive mode must be POINTS (0)")]
    ModeNotPoints,
    /// A required splat attribute is absent.
    #[error("missing required splat attribute `{0}`")]
    MissingAttribute(String),
    /// An attribute accessor is malformed.
    #[error("splat attribute `{0}` is invalid: {1}")]
    InvalidAttribute(String, &'static str),
    /// Only float attributes are supported (no quantized int variants).
    #[error(
        "splat attribute `{0}` uses a non-float component type \
         (quantized attributes are not supported)"
    )]
    NonFloatAttribute(String),
    /// Every attribute of a splat primitive must have the same element count.
    #[error("splat attribute `{attribute}` has {actual} elements, expected {expected}")]
    CountMismatch {
        /// The offending attribute semantic.
        attribute: String,
        /// The element count of `POSITION`.
        expected: usize,
        /// The element count of the offending attribute.
        actual: usize,
    },
    /// An accessor reads past the end of its buffer.
    #[error("splat attribute `{0}` overruns its buffer")]
    BufferOverrun(String),
}

/// Parse a `KHR_gaussian_splatting` glTF/GLB byte stream into a
/// [`GaussianScene`] (training-space conventions, see module docs).
///
/// `base` resolves external buffer URIs relative to the source file; pass
/// `None` for self-contained assets (GLB or data-URI buffers). Splat data
/// from every primitive carrying the extension is concatenated.
pub fn load_splat_gltf(bytes: &[u8], base: Option<&Path>) -> Result<GaussianScene, SplatGltfError> {
    let gltf = gltf::Gltf::from_slice_without_validation(bytes)?;
    let json = raw_json(bytes)?;
    let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob)?;

    let mut scene = GaussianScene::default();
    let mut found = false;
    for mesh in json["meshes"].as_array().into_iter().flatten() {
        for primitive in mesh["primitives"].as_array().into_iter().flatten() {
            let extension = &primitive["extensions"][EXTENSION];
            if !extension.is_object() {
                continue;
            }
            found = true;
            load_primitive(&gltf.document, &buffers, primitive, extension, &mut scene)?;
        }
    }
    if !found {
        return Err(SplatGltfError::NoSplatPrimitive);
    }
    Ok(scene)
}

/// Extract the raw glTF JSON document (GLB JSON chunk or the whole slice).
fn raw_json(bytes: &[u8]) -> Result<Value, SplatGltfError> {
    if bytes.starts_with(b"glTF") {
        let glb = gltf::binary::Glb::from_slice(bytes)?;
        Ok(serde_json::from_slice(&glb.json)?)
    } else {
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// Load one `KHR_gaussian_splatting` primitive, appending to `scene`.
fn load_primitive(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    primitive: &Value,
    extension: &Value,
    scene: &mut GaussianScene,
) -> Result<(), SplatGltfError> {
    if primitive["mode"].as_u64() != Some(0) {
        return Err(SplatGltfError::ModeNotPoints);
    }
    let kernel = extension["kernel"].as_str().unwrap_or_default();
    if kernel != "ellipse" {
        return Err(SplatGltfError::UnsupportedKernel(kernel.to_string()));
    }
    if let Some(sub) = extension["extensions"]
        .as_object()
        .and_then(|e| e.keys().next())
    {
        return Err(SplatGltfError::UnsupportedSubExtension(sub.clone()));
    }
    let attributes = primitive["attributes"]
        .as_object()
        .ok_or_else(|| SplatGltfError::MissingAttribute("POSITION".to_string()))?;

    let attribute = |name: &str, width: usize| -> Result<Vec<f32>, SplatGltfError> {
        read_f32_attribute(document, buffers, attributes, name, width)
    };
    let positions = attribute("POSITION", 3)?;
    let count = positions.len() / 3;
    let rotations = attribute(&attr("ROTATION"), 4)?;
    let scales = attribute(&attr("SCALE"), 3)?;
    let opacities = attribute(&attr("OPACITY"), 1)?;
    let sh_dc = attribute(&attr("SH_DEGREE_0_COEF_0"), 3)?;
    for (name, data, width) in [
        ("ROTATION", &rotations, 4),
        ("SCALE", &scales, 3),
        ("OPACITY", &opacities, 1),
        ("SH_DEGREE_0_COEF_0", &sh_dc, 3),
    ] {
        let actual = data.len() / width;
        if actual != count {
            return Err(SplatGltfError::CountMismatch {
                attribute: attr(name),
                expected: count,
                actual,
            });
        }
    }

    // Optional higher-degree SH, one RGB VEC3 per coefficient.
    let mut sh_rest_coefs: [Option<Vec<f32>>; 15] = Default::default();
    for degree in 1..=3usize {
        for n in 0..2 * degree + 1 {
            let name = attr(&format!("SH_DEGREE_{degree}_COEF_{n}"));
            if attributes.contains_key(&name) {
                let data = attribute(&name, 3)?;
                if data.len() / 3 != count {
                    return Err(SplatGltfError::CountMismatch {
                        attribute: name,
                        expected: count,
                        actual: data.len() / 3,
                    });
                }
                sh_rest_coefs[degree * degree - 1 + n] = Some(data);
            }
        }
    }

    for i in 0..count {
        scene
            .positions
            .push([positions[3 * i], positions[3 * i + 1], positions[3 * i + 2]]);
        // glTF quaternion (x, y, z, w) → training convention (w, x, y, z).
        scene.rotations.push([
            rotations[4 * i + 3],
            rotations[4 * i],
            rotations[4 * i + 1],
            rotations[4 * i + 2],
        ]);
        scene.scales.push([
            scales[3 * i].ln(),
            scales[3 * i + 1].ln(),
            scales[3 * i + 2].ln(),
        ]);
        scene.opacities.push(logit(opacities[i]));
        scene
            .sh_dc
            .push([sh_dc[3 * i], sh_dc[3 * i + 1], sh_dc[3 * i + 2]]);
        // Transpose the per-coefficient RGB vectors into the channel-blocked
        // `f_rest` layout: coefficient c of channel ch → sh_rest[ch * 15 + c].
        let mut sh_rest = [0.0f32; SH_REST_LEN];
        for (c, coef) in sh_rest_coefs.iter().enumerate() {
            if let Some(data) = coef {
                for (channel, value) in data[3 * i..3 * i + 3].iter().enumerate() {
                    sh_rest[channel * 15 + c] = *value;
                }
            }
        }
        scene.sh_rest.push(sh_rest);
    }
    Ok(())
}

/// The full attribute semantic for a splat field name.
fn attr(field: &str) -> String {
    format!("{EXTENSION}:{field}")
}

/// Inverse of the sigmoid activation used in training.
fn logit(p: f32) -> f32 {
    (p / (1.0 - p)).ln()
}

/// Decode one float attribute accessor into `count * width` tightly packed
/// f32s, honoring accessor/buffer-view offsets and `byteStride`.
fn read_f32_attribute(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    attributes: &serde_json::Map<String, Value>,
    name: &str,
    width: usize,
) -> Result<Vec<f32>, SplatGltfError> {
    let invalid = |reason: &'static str| SplatGltfError::InvalidAttribute(name.to_string(), reason);
    let index = attributes
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| SplatGltfError::MissingAttribute(name.to_string()))?;
    let accessor = document
        .accessors()
        .nth(index as usize)
        .ok_or_else(|| invalid("accessor index out of range"))?;
    if accessor.data_type() != gltf::accessor::DataType::F32 {
        return Err(SplatGltfError::NonFloatAttribute(name.to_string()));
    }
    if accessor.dimensions().multiplicity() != width {
        return Err(invalid("unexpected accessor dimensions"));
    }
    if accessor.sparse().is_some() {
        return Err(invalid("sparse accessors are not supported"));
    }
    let view = accessor
        .view()
        .ok_or_else(|| invalid("accessor has no buffer view"))?;
    let buffer = buffers
        .get(view.buffer().index())
        .ok_or_else(|| invalid("buffer index out of range"))?;

    let element = 4 * width;
    let stride = view.stride().unwrap_or(element);
    if stride < element {
        return Err(invalid("buffer view stride smaller than the element"));
    }
    let start = view.offset() + accessor.offset();
    let mut out = Vec::with_capacity(accessor.count() * width);
    for i in 0..accessor.count() {
        let at = start + i * stride;
        let bytes = buffer
            .get(at..at + element)
            .ok_or_else(|| SplatGltfError::BufferOverrun(name.to_string()))?;
        for lane in 0..width {
            out.push(f32::from_le_bytes(
                bytes[4 * lane..4 * lane + 4].try_into().unwrap(),
            ));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble a GLB container from a JSON document and a binary blob.
    fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    fn f32_bytes(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Minimal standard-alphabet base64 encoder for data-URI buffers.
    fn base64(data: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in data.chunks(3) {
            let b = |i: usize| *chunk.get(i).unwrap_or(&0);
            let n = u32::from(b(0)) << 16 | u32::from(b(1)) << 8 | u32::from(b(2));
            out.push(ALPHABET[(n >> 18) as usize & 63] as char);
            out.push(ALPHABET[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[(n >> 6) as usize & 63] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[n as usize & 63] as char
            } else {
                '='
            });
        }
        out
    }

    /// One test attribute: a tightly packed float blob plus its accessor type.
    struct Attr {
        semantic: &'static str,
        component_type: u32,
        ty: &'static str,
        count: usize,
        data: Vec<u8>,
    }

    impl Attr {
        fn float(semantic: &'static str, ty: &'static str, values: &[f32]) -> Self {
            let width = match ty {
                "SCALAR" => 1,
                "VEC3" => 3,
                "VEC4" => 4,
                _ => unreachable!(),
            };
            Self {
                semantic,
                component_type: 5126,
                ty,
                count: values.len() / width,
                data: f32_bytes(values),
            }
        }
    }

    /// Build a GLB holding one POINTS mesh primitive with the given
    /// attributes and `KHR_gaussian_splatting` extension JSON (`None` omits
    /// the extension entirely). POSITION accessors get the mandatory min/max.
    fn splat_glb(attrs: &[Attr], extension: Option<&str>) -> Vec<u8> {
        let mut bin = Vec::new();
        let mut buffer_views = String::new();
        let mut accessors = String::new();
        let mut attributes = String::new();
        for (i, attr) in attrs.iter().enumerate() {
            let offset = bin.len();
            bin.extend_from_slice(&attr.data);
            while bin.len() % 4 != 0 {
                bin.push(0);
            }
            if i > 0 {
                buffer_views.push(',');
                accessors.push(',');
                attributes.push(',');
            }
            buffer_views.push_str(&format!(
                r#"{{"buffer": 0, "byteOffset": {offset}, "byteLength": {}}}"#,
                attr.data.len()
            ));
            let min_max = if attr.semantic == "POSITION" {
                r#", "min": [0.0, 0.0, 0.0], "max": [0.0, 0.0, 0.0]"#
            } else {
                ""
            };
            accessors.push_str(&format!(
                r#"{{"bufferView": {i}, "componentType": {}, "count": {}, "type": "{}"{min_max}}}"#,
                attr.component_type, attr.count, attr.ty
            ));
            attributes.push_str(&format!(r#""{}": {i}"#, attr.semantic));
        }
        let extension = extension
            .map(|e| format!(r#", "extensions": {{"KHR_gaussian_splatting": {e}}}"#))
            .unwrap_or_default();
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "extensionsUsed": ["KHR_gaussian_splatting"],
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [{buffer_views}],
                "accessors": [{accessors}],
                "meshes": [{{"primitives": [{{"attributes": {{{attributes}}}, "mode": 0{extension}}}]}}]
            }}"#,
            bin.len()
        );
        glb(&json, &bin)
    }

    /// Known values for a two-splat fixture (degree-0 + degree-1 SH).
    fn sample_attrs() -> Vec<Attr> {
        vec![
            Attr::float("POSITION", "VEC3", &[1.0, 2.0, 3.0, -1.0, 0.5, 2.0]),
            Attr::float(
                "KHR_gaussian_splatting:ROTATION",
                "VEC4",
                &[0.0, 0.0, 0.0, 1.0, 0.1, 0.2, 0.3, 0.9],
            ),
            Attr::float(
                "KHR_gaussian_splatting:SCALE",
                "VEC3",
                &[2.0, 4.0, 0.5, 1.0, 1.0, 1.0],
            ),
            Attr::float("KHR_gaussian_splatting:OPACITY", "SCALAR", &[0.5, 0.25]),
            Attr::float(
                "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0",
                "VEC3",
                &[0.1, -0.2, 0.3, 0.0, 0.0, 0.0],
            ),
            Attr::float(
                "KHR_gaussian_splatting:SH_DEGREE_1_COEF_0",
                "VEC3",
                &[10.0, 20.0, 30.0, -10.0, -20.0, -30.0],
            ),
            Attr::float(
                "KHR_gaussian_splatting:SH_DEGREE_1_COEF_1",
                "VEC3",
                &[11.0, 21.0, 31.0, -11.0, -21.0, -31.0],
            ),
            Attr::float(
                "KHR_gaussian_splatting:SH_DEGREE_1_COEF_2",
                "VEC3",
                &[12.0, 22.0, 32.0, -12.0, -22.0, -32.0],
            ),
        ]
    }

    const KERNEL_ELLIPSE: &str = r#"{"kernel": "ellipse", "colorSpace": "srgb_rec709_display"}"#;

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn test_render_to_training_space_conversion() {
        let scene =
            load_splat_gltf(&splat_glb(&sample_attrs(), Some(KERNEL_ELLIPSE)), None).unwrap();
        assert_eq!(scene.len(), 2);

        // Positions pass through verbatim.
        assert_eq!(scene.positions[0], [1.0, 2.0, 3.0]);
        assert_eq!(scene.positions[1], [-1.0, 0.5, 2.0]);
        // Linear scale → log space.
        assert_close(scene.scales[0][0], std::f32::consts::LN_2);
        assert_close(scene.scales[0][1], 2.0 * std::f32::consts::LN_2);
        assert_close(scene.scales[0][2], -std::f32::consts::LN_2);
        assert_eq!(scene.scales[1], [0.0; 3]);
        // glTF quaternion (x, y, z, w) → (w, x, y, z).
        assert_eq!(scene.rotations[0], [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(scene.rotations[1], [0.9, 0.1, 0.2, 0.3]);
        // Opacity 0..1 → logit space.
        assert_close(scene.opacities[0], 0.0);
        assert_close(scene.opacities[1], (0.25f32 / 0.75).ln());
        // Degree-0 SH maps verbatim onto sh_dc.
        assert_eq!(scene.sh_dc[0], [0.1, -0.2, 0.3]);
    }

    #[test]
    fn test_sh_rest_transpose_is_channel_blocked() {
        let scene =
            load_splat_gltf(&splat_glb(&sample_attrs(), Some(KERNEL_ELLIPSE)), None).unwrap();
        let rest = &scene.sh_rest[0];
        // Degree-1 coefficients land at c = 0..2, channel-blocked:
        // sh_rest[ch * 15 + c] = coef value for that channel.
        assert_eq!(rest[0], 10.0); // c0 red
        assert_eq!(rest[1], 11.0); // c1 red
        assert_eq!(rest[2], 12.0); // c2 red
        assert_eq!(rest[15], 20.0); // c0 green
        assert_eq!(rest[16], 21.0);
        assert_eq!(rest[17], 22.0);
        assert_eq!(rest[30], 30.0); // c0 blue
        assert_eq!(rest[31], 31.0);
        assert_eq!(rest[32], 32.0);
        // Degrees 2 and 3 are absent → zero-filled.
        assert!(rest[3..15].iter().all(|&v| v == 0.0));
        assert!(rest[18..30].iter().all(|&v| v == 0.0));
        assert!(rest[33..45].iter().all(|&v| v == 0.0));
        // Second splat transposes its own values.
        assert_eq!(scene.sh_rest[1][0], -10.0);
        assert_eq!(scene.sh_rest[1][17], -22.0);
    }

    #[test]
    fn test_missing_higher_degrees_are_zero_filled() {
        let attrs: Vec<Attr> = sample_attrs().into_iter().take(5).collect();
        let scene = load_splat_gltf(&splat_glb(&attrs, Some(KERNEL_ELLIPSE)), None).unwrap();
        assert_eq!(scene.len(), 2);
        assert_eq!(scene.sh_dc[0], [0.1, -0.2, 0.3]);
        assert!(
            scene
                .sh_rest
                .iter()
                .all(|rest| rest.iter().all(|&v| v == 0.0))
        );
    }

    #[test]
    fn test_byte_strided_buffer_view_is_decoded() {
        // POSITION (VEC3) + OPACITY (SCALAR) interleaved at stride 16.
        let mut interleaved = Vec::new();
        for (pos, opacity) in [([1.0f32, 2.0, 3.0], 0.5f32), ([4.0, 5.0, 6.0], 0.25)] {
            interleaved.extend_from_slice(&f32_bytes(&pos));
            interleaved.extend_from_slice(&f32_bytes(&[opacity]));
        }
        let dc = f32_bytes(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        let rot = f32_bytes(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        let scale = f32_bytes(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        let mut bin = interleaved;
        let dc_off = bin.len();
        bin.extend_from_slice(&dc);
        let rot_off = bin.len();
        bin.extend_from_slice(&rot);
        let scale_off = bin.len();
        bin.extend_from_slice(&scale);
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 32, "byteStride": 16}},
                    {{"buffer": 0, "byteOffset": {dc_off}, "byteLength": 24}},
                    {{"buffer": 0, "byteOffset": {rot_off}, "byteLength": 32}},
                    {{"buffer": 0, "byteOffset": {scale_off}, "byteLength": 24}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 2, "type": "VEC3",
                      "min": [1.0, 2.0, 3.0], "max": [4.0, 5.0, 6.0]}},
                    {{"bufferView": 0, "byteOffset": 12, "componentType": 5126, "count": 2, "type": "SCALAR"}},
                    {{"bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3"}},
                    {{"bufferView": 2, "componentType": 5126, "count": 2, "type": "VEC4"}},
                    {{"bufferView": 3, "componentType": 5126, "count": 2, "type": "VEC3"}}
                ],
                "meshes": [{{"primitives": [{{
                    "attributes": {{
                        "POSITION": 0,
                        "KHR_gaussian_splatting:OPACITY": 1,
                        "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0": 2,
                        "KHR_gaussian_splatting:ROTATION": 3,
                        "KHR_gaussian_splatting:SCALE": 4
                    }},
                    "mode": 0,
                    "extensions": {{"KHR_gaussian_splatting": {KERNEL_ELLIPSE}}}
                }}]}}]
            }}"#,
            bin.len()
        );
        let scene = load_splat_gltf(&glb(&json, &bin), None).unwrap();
        assert_eq!(scene.positions[1], [4.0, 5.0, 6.0]);
        assert_close(scene.opacities[0], 0.0);
        assert_close(scene.opacities[1], (0.25f32 / 0.75).ln());
        assert_eq!(scene.sh_dc[1], [0.4, 0.5, 0.6]);
    }

    #[test]
    fn test_plain_gltf_with_data_uri_buffer() {
        // Covers the non-GLB branch: JSON slice + base64 data-URI buffer.
        let mut bin = f32_bytes(&[1.0, 2.0, 3.0]); // POSITION
        let rot_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[0.0, 0.0, 0.0, 1.0]));
        let scale_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[2.0, 2.0, 2.0]));
        let opacity_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[0.5]));
        let dc_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[0.1, 0.2, 0.3]));
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}, "uri": "data:application/octet-stream;base64,{}"}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 12}},
                    {{"buffer": 0, "byteOffset": {rot_off}, "byteLength": 16}},
                    {{"buffer": 0, "byteOffset": {scale_off}, "byteLength": 12}},
                    {{"buffer": 0, "byteOffset": {opacity_off}, "byteLength": 4}},
                    {{"buffer": 0, "byteOffset": {dc_off}, "byteLength": 12}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
                      "min": [1.0, 2.0, 3.0], "max": [1.0, 2.0, 3.0]}},
                    {{"bufferView": 1, "componentType": 5126, "count": 1, "type": "VEC4"}},
                    {{"bufferView": 2, "componentType": 5126, "count": 1, "type": "VEC3"}},
                    {{"bufferView": 3, "componentType": 5126, "count": 1, "type": "SCALAR"}},
                    {{"bufferView": 4, "componentType": 5126, "count": 1, "type": "VEC3"}}
                ],
                "meshes": [{{"primitives": [{{
                    "attributes": {{
                        "POSITION": 0,
                        "KHR_gaussian_splatting:ROTATION": 1,
                        "KHR_gaussian_splatting:SCALE": 2,
                        "KHR_gaussian_splatting:OPACITY": 3,
                        "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0": 4
                    }},
                    "mode": 0,
                    "extensions": {{"KHR_gaussian_splatting": {KERNEL_ELLIPSE}}}
                }}]}}]
            }}"#,
            bin.len(),
            base64(&bin)
        );
        let scene = load_splat_gltf(json.as_bytes(), None).unwrap();
        assert_eq!(scene.len(), 1);
        assert_eq!(scene.positions[0], [1.0, 2.0, 3.0]);
        assert_close(scene.scales[0][0], std::f32::consts::LN_2);
        assert_eq!(scene.sh_dc[0], [0.1, 0.2, 0.3]);
    }

    #[test]
    fn test_quantized_attribute_is_rejected() {
        let mut attrs = sample_attrs();
        attrs[2] = Attr {
            semantic: "KHR_gaussian_splatting:SCALE",
            component_type: 5121, // unsigned byte
            ty: "VEC3",
            count: 2,
            data: vec![255; 6],
        };
        assert!(matches!(
            load_splat_gltf(&splat_glb(&attrs, Some(KERNEL_ELLIPSE)), None),
            Err(SplatGltfError::NonFloatAttribute(name)) if name == "KHR_gaussian_splatting:SCALE"
        ));
    }

    #[test]
    fn test_missing_splat_extension_is_rejected() {
        let attrs = [Attr::float("POSITION", "VEC3", &[0.0, 0.0, 0.0])];
        assert!(matches!(
            load_splat_gltf(&splat_glb(&attrs, None), None),
            Err(SplatGltfError::NoSplatPrimitive)
        ));
    }

    #[test]
    fn test_unsupported_kernel_is_rejected() {
        assert!(matches!(
            load_splat_gltf(
                &splat_glb(&sample_attrs(), Some(r#"{"kernel": "cube", "colorSpace": "srgb_rec709_display"}"#)),
                None
            ),
            Err(SplatGltfError::UnsupportedKernel(kernel)) if kernel == "cube"
        ));
    }

    #[test]
    fn test_compression_sub_extension_is_rejected() {
        let ext = r#"{"kernel": "ellipse", "colorSpace": "srgb_rec709_display",
            "extensions": {"KHR_gaussian_splatting_compression_spz": {}}}"#;
        assert!(matches!(
            load_splat_gltf(&splat_glb(&sample_attrs(), Some(ext)), None),
            Err(SplatGltfError::UnsupportedSubExtension(_))
        ));
    }

    #[test]
    fn test_missing_required_attribute_is_rejected() {
        let attrs: Vec<Attr> = sample_attrs()
            .into_iter()
            .filter(|a| a.semantic != "KHR_gaussian_splatting:OPACITY")
            .collect();
        assert!(matches!(
            load_splat_gltf(&splat_glb(&attrs, Some(KERNEL_ELLIPSE)), None),
            Err(SplatGltfError::MissingAttribute(name)) if name == "KHR_gaussian_splatting:OPACITY"
        ));
    }

    #[test]
    fn test_non_points_mode_is_rejected() {
        let glb = splat_glb(&sample_attrs(), Some(KERNEL_ELLIPSE));
        // Flip the primitive mode from POINTS (0) to TRIANGLES (4).
        let json_str = r#""mode": 0"#;
        let mut bytes = glb;
        let window = bytes
            .windows(json_str.len())
            .position(|w| w == json_str.as_bytes())
            .unwrap();
        bytes[window + json_str.len() - 1] = b'4';
        assert!(matches!(
            load_splat_gltf(&bytes, None),
            Err(SplatGltfError::ModeNotPoints)
        ));
    }
}

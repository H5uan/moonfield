//! Generate `assets/models/synth_splats.glb`: the synthetic
//! `KHR_gaussian_splatting` reference asset for the splat forward pass's
//! editor acceptance — an 8×8×8 grid of Gaussians centered at the origin,
//! colored by grid position (RGB = normalized xyz), moderate world-space
//! scales, near-full opacity. Unambiguous on screen: a colored 3D cloud.
//!
//! The file holds the loader's input dialect — linear scales, linear
//! opacities, (x, y, z, w) quaternions, raw SH DC coefficients used directly
//! as linear RGB (the forward pass's `eval_color` returns `sh_dc`
//! unchanged); the loader converts to training space on read. Run:
//!
//! ```sh
//! cargo run -p moonfield-render-feature --example gen_splat_asset --features splat
//! ```
//!
//! The example round-trips its own output through `load_splat_gltf` before
//! exiting: the file must parse through the engine's loader to count as
//! generated.

use std::path::PathBuf;

use moonfield_render_feature::splat::io::gltf::load_splat_gltf;

/// Grid side length; N³ Gaussians.
const N: usize = 8;
/// Grid step in world units (the grid spans ±(N-1)/2 × STEP).
const STEP: f32 = 0.45;
/// Linear per-axis scale written to the file.
const SCALE: f32 = 0.09;
/// Linear opacity written to the file.
const OPACITY: f32 = 0.98;

/// The file-side splat values (render space: linear scale/opacity, xyzw
/// quaternions), one entry per Gaussian.
struct SplatData {
    positions: Vec<[f32; 3]>,
    rotations: Vec<[f32; 4]>,
    scales: Vec<[f32; 3]>,
    opacities: Vec<f32>,
    sh_dc: Vec<[f32; 3]>,
}

/// The synthetic cloud: an N×N×N grid, color = normalized grid position.
fn synthetic_splats() -> SplatData {
    let mut data = SplatData {
        positions: Vec::with_capacity(N * N * N),
        rotations: Vec::with_capacity(N * N * N),
        scales: Vec::with_capacity(N * N * N),
        opacities: Vec::with_capacity(N * N * N),
        sh_dc: Vec::with_capacity(N * N * N),
    };
    let center = (N as f32 - 1.0) * 0.5;
    let extent = (N - 1) as f32;
    for z in 0..N {
        for y in 0..N {
            for x in 0..N {
                let (xf, yf, zf) = (x as f32, y as f32, z as f32);
                data.positions.push([
                    (xf - center) * STEP,
                    (yf - center) * STEP,
                    (zf - center) * STEP,
                ]);
                data.rotations.push([0.0, 0.0, 0.0, 1.0]);
                data.scales.push([SCALE; 3]);
                data.opacities.push(OPACITY);
                data.sh_dc.push([xf / extent, yf / extent, zf / extent]);
            }
        }
    }
    data
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Serialize as a minimal GLB: one buffer, one POINTS primitive carrying the
/// `KHR_gaussian_splatting` extension with the ellipse kernel — exactly the
/// dialect `load_splat_gltf` accepts.
fn build_glb(data: &SplatData) -> Vec<u8> {
    let flat = |v: &[[f32; 3]]| -> Vec<f32> { v.iter().flatten().copied().collect() };
    let positions = flat(&data.positions);
    let scales = flat(&data.scales);
    let sh_dc = flat(&data.sh_dc);
    let rotations: Vec<f32> = data.rotations.iter().flatten().copied().collect();
    let blobs: [(&str, u32, &[f32]); 5] = [
        ("POSITION", 3, &positions),
        ("KHR_gaussian_splatting:ROTATION", 4, &rotations),
        ("KHR_gaussian_splatting:SCALE", 3, &scales),
        ("KHR_gaussian_splatting:OPACITY", 1, &data.opacities),
        ("KHR_gaussian_splatting:SH_DEGREE_0_COEF_0", 3, &sh_dc),
    ];
    let (min, max) = data.positions.iter().fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(mut min, mut max), p| {
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
            (min, max)
        },
    );

    let count = data.positions.len();
    let mut bin = Vec::new();
    let mut buffer_views = String::new();
    let mut accessors = String::new();
    let mut attributes = String::new();
    for (i, (semantic, width, values)) in blobs.iter().enumerate() {
        let offset = bin.len();
        bin.extend_from_slice(&f32_bytes(values));
        if i > 0 {
            buffer_views.push(',');
            accessors.push(',');
            attributes.push(',');
        }
        buffer_views.push_str(&format!(
            r#"{{"buffer": 0, "byteOffset": {offset}, "byteLength": {}}}"#,
            values.len() * 4
        ));
        let ty = ["", "SCALAR", "", "VEC3", "VEC4"][*width as usize];
        // POSITION accessors carry the mandatory min/max.
        let min_max = if *semantic == "POSITION" {
            format!(r#", "min": {min:?}, "max": {max:?}"#)
        } else {
            String::new()
        };
        accessors.push_str(&format!(
            r#"{{"bufferView": {i}, "componentType": 5126, "count": {count}, "type": "{ty}"{min_max}}}"#
        ));
        attributes.push_str(&format!(r#""{semantic}": {i}"#));
    }
    let json = format!(
        r#"{{
    "asset": {{"version": "2.0", "generator": "moonfield gen_splat_asset"}},
    "extensionsUsed": ["KHR_gaussian_splatting"],
    "buffers": [{{"byteLength": {}}}],
    "bufferViews": [{buffer_views}],
    "accessors": [{accessors}],
    "meshes": [{{"primitives": [{{
        "attributes": {{{attributes}}},
        "mode": 0,
        "extensions": {{"KHR_gaussian_splatting": {{"kernel": "ellipse", "colorSpace": "srgb_rec709_display"}}}}
    }}]}}]
}}"#,
        bin.len()
    );

    // GLB container: header, padded JSON chunk, padded BIN chunk.
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
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

fn main() {
    let data = synthetic_splats();
    let bytes = build_glb(&data);
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "assets", "models"]
        .into_iter()
        .collect();
    let path = path.join("synth_splats.glb");
    std::fs::write(&path, &bytes).expect("write synth_splats.glb");
    println!(
        "wrote {} ({} bytes, {} Gaussians)",
        path.display(),
        bytes.len(),
        data.positions.len()
    );

    // Round-trip through the engine's own loader: the file is only done
    // when it parses and the training-space values check out.
    let written = std::fs::read(&path).expect("read back synth_splats.glb");
    let scene = load_splat_gltf(&written, None).expect("generated file must load");
    assert_eq!(scene.len(), N * N * N);
    let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-6, "{a} != {b}");
    close(scene.scales()[0][0], SCALE.ln());
    close(scene.opacities()[0], (OPACITY / (1.0 - OPACITY)).ln());
    assert_eq!(scene.rotations()[0], [1.0, 0.0, 0.0, 0.0]); // xyzw → wxyz
    // First grid corner is black, the opposite white, one mid axis red.
    assert_eq!(scene.sh_dc()[0], [0.0, 0.0, 0.0]);
    let last = scene.sh_dc()[N * N * N - 1];
    close(last[0], 1.0);
    close(last[1], 1.0);
    close(last[2], 1.0);
    let half = (N as f32 - 1.0) * 0.5 * STEP;
    close(scene.positions()[0][0], -half);
    close(scene.positions()[N * N * N - 1][0], half);
    println!("round-trip through load_splat_gltf: ok");
}

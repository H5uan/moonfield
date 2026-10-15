//! GPU acceptance for the splat forward chain (`splat::forward_pass`):
//! three depth-stacked Gaussians plus one off-screen cull case render into
//! the per-view RGBA16F splat image through the real app (RenderPlugin +
//! RenderFeaturePlugin + SplatForwardPlugin), and the readback asserts the
//! semantics — occlusion order, back-only coverage, transparent corners, and
//! that the culled Gaussian leaked nowhere.
//!
//! Headless machines (no Vulkan driver) skip; with a driver this test must
//! actually run.

use moonfield_app::App;
use moonfield_app::prelude::Entity;
use moonfield_asset::{AssetServer, Assets};
use moonfield_camera::{Camera, PrimaryCamera};
use moonfield_gaussian::GaussianScene;
use moonfield_math::GlobalTransform;
use moonfield_render_core::{MainEntity, RenderPlugin, RenderTargetSizes, ViewTargets};
use moonfield_render_feature::splat::cloud::{SplatCloud, SplatCloudHandle};
use moonfield_render_feature::splat::forward_pass::{
    SplatForwardPlugin, SplatViewArtifacts, splat_composite_shader, splat_forward_shader,
};
use moonfield_render_feature::splat::sort_pass::splat_sort_shader;
use moonfield_render_feature::{RenderFeaturePlugin, shader::PipelineShaders};
use moonfield_rhi::RenderDevice;
use moonfield_shader::{Shader, SlangLoader};

/// The offscreen view size: 4×4 tiles.
const SIZE: u32 = 64;

/// The test camera's clear color — what an uncovered view-target pixel must
/// keep through the composite.
const CLEAR_COLOR: [f32; 4] = [0.1, 0.2, 0.3, 1.0];

/// f16 (IEEE 754 half) → f32, for the RGBA16F readback.
fn f16_to_f32(bits: u16) -> f32 {
    let bits = u32::from(bits);
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = ((bits >> 10) & 0x1f) as i32;
    let mantissa = (bits & 0x3ff) as f32;
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        e => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

/// One RGBA16F texel as four f32s.
fn pixel(pixels: &[u8], x: u32, y: u32) -> [f32; 4] {
    let base = ((y * SIZE + x) * 8) as usize;
    std::array::from_fn(|c| {
        f16_to_f32(u16::from_le_bytes([
            pixels[base + c * 2],
            pixels[base + c * 2 + 1],
        ]))
    })
}

/// The test scene, in the engine's world convention (camera at the origin
/// looking down -Z): three depth-stacked centered Gaussians — small red in
/// front (z=3), small green mid (z=6), large blue in back (z=9) — and a
/// yellow one centered far off-screen with a footprint that covers no tile
/// (must be culled by its tile rectangle; had it survived, the clamped
/// rectangle would paint the right edge). All fully opaque (logit 8).
fn test_scene() -> GaussianScene {
    GaussianScene::new(
        vec![
            [0.0, 0.0, -3.0],
            [0.0, 0.0, -6.0],
            [0.0, 0.0, -9.0],
            [50.0, 0.0, -10.0],
        ],
        vec![
            [f32::ln(0.3); 3],
            [f32::ln(0.6); 3],
            [f32::ln(2.0); 3],
            [0.0; 3],
        ],
        vec![[1.0, 0.0, 0.0, 0.0]; 4],
        vec![8.0; 4],
        vec![
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 1.0, 0.0],
        ],
        vec![[0.0; 45]; 4],
    )
    .expect("valid test scene")
}

/// Build the app and render two frames: frame 1 builds the pipelines and
/// per-view artifacts (and may already render), frame 2 records against
/// warm state. Returns `None` when no Vulkan device exists (skip).
fn rendered_app() -> Option<(App, Entity)> {
    let mut app = App::new();
    app.add_plugin(RenderPlugin);
    app.add_plugin(RenderFeaturePlugin);
    app.add_plugin(SplatForwardPlugin);

    // The plugins' startup systems load these same shaders on their own;
    // pushing the requests here keeps the test's handles explicit.
    let mut server = AssetServer::default();
    server.register_loader(SlangLoader);
    let mut assets = app
        .world_mut()
        .get_resource_mut::<Assets<Shader>>()
        .expect("Assets<Shader> registered by RenderFeaturePlugin");
    let forward = server
        .load(
            &mut assets,
            &moonfield_asset::assets_dir().join("shaders/gaussian.slang"),
        )
        .expect("load gaussian.slang");
    let sort = server
        .load(
            &mut assets,
            &moonfield_asset::assets_dir().join("shaders/util/radix_sort.slang"),
        )
        .expect("load radix_sort.slang");
    let composite = server
        .load(
            &mut assets,
            &moonfield_asset::assets_dir().join("shaders/gs/composite.slang"),
        )
        .expect("load composite.slang");
    drop(assets);
    let mut requests = app
        .world_mut()
        .get_resource_mut::<PipelineShaders>()
        .expect("PipelineShaders registered by RenderFeaturePlugin");
    requests.push(splat_forward_shader(forward));
    requests.push(splat_sort_shader(sort));
    requests.push(splat_composite_shader(composite));
    drop(requests);

    let cloud = app
        .world_mut()
        .get_resource_mut::<Assets<SplatCloud>>()
        .expect("Assets<SplatCloud> registered by RenderFeaturePlugin")
        .add(SplatCloud::new(test_scene(), None));
    app.world_mut()
        .spawn((SplatCloudHandle(cloud), GlobalTransform::IDENTITY));
    let camera = app.world_mut().spawn((
        Camera {
            clear_color: CLEAR_COLOR,
            ..Camera::default()
        },
        PrimaryCamera,
        GlobalTransform::IDENTITY,
    ));
    // The viewport size a panel would report, so the pooled target (and the
    // splat artifacts) come out at 64×64 instead of the 1280×720 default.
    app.render_world_mut().insert_resource(RenderTargetSizes(
        [(MainEntity(camera), (SIZE, SIZE))].into_iter().collect(),
    ));

    app.render();
    if !app.render_world().contains_resource::<RenderDevice>() {
        moonfield_rhi::testing::skip_gpu("no Vulkan device available");
        return None;
    }
    app.render();
    Some((app, camera))
}

#[test]
fn splat_forward_blends_depth_ordered() {
    let Some((app, camera)) = rendered_app() else {
        return;
    };
    let device = app
        .render_world()
        .get_resource::<RenderDevice>()
        .expect("render device")
        .clone();
    device.device().wait_idle().expect("wait idle");

    let artifacts = app
        .render_world()
        .get_resource::<SplatViewArtifacts>()
        .expect("SplatViewArtifacts registered by SplatForwardPlugin");
    let view = artifacts
        .get(MainEntity(camera))
        .expect("artifacts built for the camera");
    assert_eq!(view.extent(), (SIZE, SIZE));
    let pixels = view.image().read_pixels(device.device()).expect("readback");
    assert_eq!(pixels.len(), (SIZE * SIZE * 8) as usize);

    // The front red Gaussian occludes the stacked center.
    let center = pixel(&pixels, 32, 32);
    assert!(
        center[0] > 0.9 && center[1] < 0.2 && center[2] < 0.2 && center[3] > 0.9,
        "center pixel must be opaque red (front occludes), got {center:?}"
    );

    // Outside the small front/mid footprints, only the large back blue
    // covers — depth ordering must not leak red/green there.
    let back_only = pixel(&pixels, 32, 46);
    assert!(
        back_only[2] > 0.3 && back_only[0] < 0.15 && back_only[1] < 0.15,
        "back-only pixel must be blue-dominated, got {back_only:?}"
    );

    // Past every footprint the tile stays empty: transparent black.
    let corner = pixel(&pixels, 2, 2);
    assert!(
        corner[3] < 0.01,
        "corner pixel must be transparent, got {corner:?}"
    );

    // The culled yellow Gaussian (off-screen center, footprint covers no
    // tile) must have contributed nowhere — a leak would clamp onto the
    // grid and paint yellow down the right edge.
    let yellow_pixels = (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&pixels, x, y))
        .filter(|px| px[0] > 0.5 && px[1] > 0.5)
        .count();
    assert_eq!(
        yellow_pixels, 0,
        "the culled Gaussian leaked into the image"
    );
}

/// The composite step: the splat image lands in the view target. Read the
/// pooled `OffscreenTarget` (BGRA8) back and check the over math against
/// hand-computed Reinhard values.
#[test]
fn splat_composite_writes_view_target() {
    let Some((app, camera)) = rendered_app() else {
        return;
    };
    let device = app
        .render_world()
        .get_resource::<RenderDevice>()
        .expect("render device")
        .clone();
    device.device().wait_idle().expect("wait idle");

    let targets = app
        .render_world()
        .get_resource::<ViewTargets>()
        .expect("ViewTargets");
    let target = targets
        .get(MainEntity(camera))
        .expect("pooled target for the camera");
    let pixels = target.read_pixels(device.device()).expect("readback");
    assert_eq!(pixels.len(), (SIZE * SIZE * 4) as usize);
    // BGRA8, row-major.
    let px = |x: u32, y: u32| -> [u8; 4] {
        let base = ((y * SIZE + x) * 4) as usize;
        pixels[base..base + 4].try_into().unwrap()
    };
    let near = |got: u8, expected: f32, what: &str| {
        let expected_byte = (expected * 255.0).round() as i32;
        assert!(
            (i32::from(got) - expected_byte).abs() <= 2,
            "{what}: got {got}, expected ~{expected_byte}"
        );
    };

    // Center: the front red Gaussian is ~opaque (splat alpha ≈ 1), so the
    // target ≈ Reinhard(premultiplied splat color). The blend accumulates
    // red a=0.99 (capped) then green ≈ 0.0099: rgb ≈ (0.99, 0.0099, 0),
    // and c/(1+c) maps them to ≈ 0.497 and ≈ 0.0098.
    let [b, g, r, a] = px(32, 32);
    near(r, 0.99 / 1.99, "center red (tonemapped)");
    near(g, 0.0099 / 1.0099, "center green (leak-through)");
    near(b, 0.0, "center blue");
    assert_eq!(a, 255, "target alpha stays the clear alpha");

    // Uncovered corner: the splat image is transparent there, so the target
    // keeps the clear color (the "scene underneath survives" half of the
    // composite contract).
    let [b, g, r, a] = px(2, 2);
    near(b, CLEAR_COLOR[2], "corner blue (clear)");
    near(g, CLEAR_COLOR[1], "corner green (clear)");
    near(r, CLEAR_COLOR[0], "corner red (clear)");
    assert_eq!(a, 255);

    // Back-only pixel: blue-dominated, composited over the dimmed clear
    // color. Hand-computed: splat rgb ≈ (0.039, 0.037, 0.48) at alpha ≈ 0.56;
    // Reinhard gives ≈ (0.038, 0.036, 0.33), and 0.44 × clear adds
    // ≈ (0.044, 0.087, 0.131) — blue lands near 0.46, the others under 0.13.
    let [b, g, r, _] = px(32, 46);
    assert!(
        i32::from(b) > 100 && b > 2 * r && b > 2 * g,
        "back-only pixel must composite blue over the clear color, got BGRA [{b}, {g}, {r}]"
    );
}

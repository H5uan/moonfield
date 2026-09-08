//! GPU-vs-CPU verification of the shared Gaussian math
//! (`assets/shaders/gs/gaussian.slang`): `cov3d`, `project`, and
//! `eval_color` over 64 seeded Gaussians, compared per component against an
//! independent glam reference. The wrapper kernel is a source string whose
//! module name is a virtual path in `assets/shaders/gs/`; the `import
//! gaussian` resolves through that path hint (see the rhi `compile_source`
//! docs), so no fixture file exists on disk.
//!
//! The one contract point between the sides: rotations are stored
//! (w, x, y, z) — the `GaussianScene`/3DGS-reference layout — while glam's
//! `Quat::from_xyzw` takes (x, y, z, w); the reference converts at the
//! boundary.

use moonfield_math::{EulerRot, Mat3, Quat, Vec3};
use moonfield_rhi::{
    CommandBuffer, CommandBufferUsage, CommandPool, ComputePipeline, Device, GpuAllocation, GpuPtr,
    Instance, Memory, ShaderModule,
};

/// Number of seeded Gaussians.
const N: usize = 64;
/// Scalars per Gaussian: mean.xyz, log_scale.xyz, rotation (w, x, y, z),
/// logit_opacity, sh_dc.rgb.
const PPG: usize = 14;
/// View inputs: W column-major (9), cam_pos (3), fx, fy, pp (2).
const VIEW_FLOATS: usize = 16;
/// Output per Gaussian: covariance upper triangle (6), screen (2), conic
/// (3), depth (1), color (3).
const OUT_PPG: usize = 15;

/// Output component names, in the kernel's write order.
const COMPONENTS: [&str; OUT_PPG] = [
    "cov00", "cov01", "cov02", "cov11", "cov12", "cov22", "screen.x", "screen.y", "conic.x",
    "conic.y", "conic.z", "depth", "color.x", "color.y", "color.z",
];

/// One Gaussian per thread: load from `params` (`[N][14]`), evaluate the
/// shared math, write `[N][15]` results.
const WRAPPER: &str = r#"
import gaussian;

[shader("compute")]
[numthreads(64, 1, 1)]
void eval(uint3 tid : SV_DispatchThreadID,
          Ptr<float, Access.Read> params,
          Ptr<float, Access.Read> view_buf,
          Ptr<float, Access.ReadWrite> out_buf)
{
    uint i = tid.x;
    if (i >= 64) return;

    uint o = i * 14;
    Gaussian3D g;
    g.mean = float3(params[o + 0], params[o + 1], params[o + 2]);
    g.log_scale = float3(params[o + 3], params[o + 4], params[o + 5]);
    g.rotation = float4(params[o + 6], params[o + 7], params[o + 8], params[o + 9]);
    g.logit_opacity = params[o + 10];
    g.sh_dc = float3(params[o + 11], params[o + 12], params[o + 13]);

    SplatView view;
    // view_buf holds W column-major (glam `to_cols_array`); the matrix
    // constructor takes rows, so transpose back.
    view.w = transpose(float3x3(
        float3(view_buf[0], view_buf[1], view_buf[2]),
        float3(view_buf[3], view_buf[4], view_buf[5]),
        float3(view_buf[6], view_buf[7], view_buf[8])));
    view.cam_pos = float3(view_buf[9], view_buf[10], view_buf[11]);
    view.fx = view_buf[12];
    view.fy = view_buf[13];
    view.pp = float2(view_buf[14], view_buf[15]);

    float3x3 cov = cov3d(g);
    ProjectedSplat p = project(g, view);
    float3 color = eval_color(float3(0.0, 0.0, 1.0), g);

    uint q = i * 15;
    out_buf[q + 0] = cov[0][0];
    out_buf[q + 1] = cov[0][1];
    out_buf[q + 2] = cov[0][2];
    out_buf[q + 3] = cov[1][1];
    out_buf[q + 4] = cov[1][2];
    out_buf[q + 5] = cov[2][2];
    out_buf[q + 6] = p.screen.x;
    out_buf[q + 7] = p.screen.y;
    out_buf[q + 8] = p.conic.x;
    out_buf[q + 9] = p.conic.y;
    out_buf[q + 10] = p.conic.z;
    out_buf[q + 11] = p.depth;
    out_buf[q + 12] = color.x;
    out_buf[q + 13] = color.y;
    out_buf[q + 14] = color.z;
}
"#;

/// Deterministic xorshift32, the same shape as the ml tests.
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1 << 24) as f32
    }
}

/// One seeded Gaussian in the `GaussianScene` conventions.
struct TestGaussian {
    mean: [f32; 3],
    log_scale: [f32; 3],
    /// (w, x, y, z), unit.
    rotation: [f32; 4],
    logit_opacity: f32,
    sh_dc: [f32; 3],
}

/// Fixed camera: a mild rotation, the camera behind the scene, distinct
/// focal lengths (an axis swap would show immediately).
struct TestView {
    w: Mat3,
    cam_pos: Vec3,
    fx: f32,
    fy: f32,
    pp: [f32; 2],
}

fn build_gaussians() -> Vec<TestGaussian> {
    let mut rng = Rng(0x9E37_79B9);
    (0..N)
        .map(|_| {
            let mut rotation = [
                rng.next() * 2.0 - 1.0,
                rng.next() * 2.0 - 1.0,
                rng.next() * 2.0 - 1.0,
                rng.next() * 2.0 - 1.0,
            ];
            let len = rotation.iter().map(|r| r * r).sum::<f32>().sqrt();
            for r in &mut rotation {
                *r /= len;
            }
            TestGaussian {
                mean: [
                    (rng.next() * 2.0 - 1.0) * 1.5,
                    (rng.next() * 2.0 - 1.0) * 1.5,
                    rng.next() * 3.0,
                ],
                log_scale: [
                    -3.0 + rng.next() * 2.5,
                    -3.0 + rng.next() * 2.5,
                    -3.0 + rng.next() * 2.5,
                ],
                rotation,
                logit_opacity: rng.next() * 4.0 - 2.0,
                sh_dc: [rng.next(), rng.next(), rng.next()],
            }
        })
        .collect()
}

fn build_view() -> TestView {
    TestView {
        w: Mat3::from_quat(Quat::from_euler(EulerRot::XYZ, 0.12, -0.2, 0.08)),
        cam_pos: Vec3::new(0.3, -0.4, -3.0),
        fx: 480.0,
        fy: 530.0,
        pp: [320.0, 240.0],
    }
}

/// The reference math, computed through glam only: quaternion and matrix
/// products go through glam's constructors, never a transcription of the
/// Slang formulas.
fn reference(g: &TestGaussian, view: &TestView) -> [f32; OUT_PPG] {
    // (w, x, y, z) storage → glam's (x, y, z, w) argument order.
    let q = Quat::from_xyzw(g.rotation[1], g.rotation[2], g.rotation[3], g.rotation[0]);
    let r = Mat3::from_quat(q);
    let s = Vec3::new(
        g.log_scale[0].exp(),
        g.log_scale[1].exp(),
        g.log_scale[2].exp(),
    );
    let a = Mat3::from_cols(r.col(0) * s.x, r.col(1) * s.y, r.col(2) * s.z);
    let sigma = a * a.transpose();

    let t = view.w * (Vec3::from_array(g.mean) - view.cam_pos);
    let inv_z = 1.0 / t.z;
    // T = J·W as two rows; J is the perspective Jacobian at t.
    let j0 = Vec3::new(view.fx * inv_z, 0.0, -view.fx * t.x * inv_z * inv_z);
    let j1 = Vec3::new(0.0, view.fy * inv_z, -view.fy * t.y * inv_z * inv_z);
    let t0 = Vec3::new(
        j0.dot(view.w.col(0)),
        j0.dot(view.w.col(1)),
        j0.dot(view.w.col(2)),
    );
    let t1 = Vec3::new(
        j1.dot(view.w.col(0)),
        j1.dot(view.w.col(1)),
        j1.dot(view.w.col(2)),
    );
    let c00 = t0.dot(sigma * t0) + 0.3;
    let c01 = t0.dot(sigma * t1);
    let c11 = t1.dot(sigma * t1) + 0.3;
    let det = c00 * c11 - c01 * c01;

    [
        sigma.x_axis.x,
        sigma.x_axis.y,
        sigma.x_axis.z,
        sigma.y_axis.y,
        sigma.y_axis.z,
        sigma.z_axis.z,
        view.fx * t.x / t.z + view.pp[0],
        view.fy * t.y / t.z + view.pp[1],
        c11 / det,
        -c01 / det,
        c00 / det,
        t.z,
        g.sh_dc[0],
        g.sh_dc[1],
        g.sh_dc[2],
    ]
}

/// Relative comparison with an absolute floor for near-zero references
/// (conic components of large projected Gaussians).
fn assert_close(i: usize, name: &str, got: f32, reference: f32) {
    let tolerance = 1e-4 * reference.abs().max(1e-3);
    assert!(
        (got - reference).abs() <= tolerance,
        "Gaussian {i} {name}: gpu = {got:.6}, reference = {reference:.6}"
    );
}

/// Write `values` through a host-visible allocation's persistent mapping.
fn write_floats(alloc: &GpuAllocation, values: &[f32]) {
    // SAFETY: the allocation is host-visible, persistently mapped, and sized
    // `values.len()` floats.
    unsafe {
        std::ptr::copy_nonoverlapping(
            values.as_ptr(),
            alloc
                .host()
                .expect("allocation must have a host view")
                .typed::<f32>(),
            values.len(),
        );
    }
}

/// Read `len` floats back from a host-visible allocation.
fn read_floats(alloc: &GpuAllocation, len: usize) -> Vec<f32> {
    // SAFETY: host-visible and sized `len` floats; read after
    // `submit_and_wait`, so the GPU is done writing.
    unsafe {
        std::slice::from_raw_parts(
            alloc
                .host()
                .expect("allocation must have a host view")
                .typed::<f32>(),
            len,
        )
        .to_vec()
    }
}

/// Push a list of root pointers as consecutive u64s, matching the Slang
/// entry parameters in declaration order (the pure-pointer-kernel layout).
fn push_roots(cmd: &CommandBuffer, ptrs: &[GpuPtr]) {
    let mut bytes = Vec::with_capacity(ptrs.len() * 8);
    for ptr in ptrs {
        bytes.extend_from_slice(&ptr.as_raw().to_le_bytes());
    }
    cmd.push_data(0, &bytes);
}

#[test]
fn gaussian_math_matches_glam_reference() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return;
        }
    };
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };

    let gaussians = build_gaussians();
    let view = build_view();

    let mut params_cpu = Vec::with_capacity(N * PPG);
    for g in &gaussians {
        params_cpu.extend_from_slice(&g.mean);
        params_cpu.extend_from_slice(&g.log_scale);
        params_cpu.extend_from_slice(&g.rotation);
        params_cpu.push(g.logit_opacity);
        params_cpu.extend_from_slice(&g.sh_dc);
    }
    let mut view_cpu = Vec::with_capacity(VIEW_FLOATS);
    view_cpu.extend_from_slice(&view.w.to_cols_array());
    view_cpu.extend_from_slice(&view.cam_pos.to_array());
    view_cpu.push(view.fx);
    view_cpu.push(view.fy);
    view_cpu.extend_from_slice(&view.pp);

    let params = GpuAllocation::new(
        &device,
        (N * PPG * size_of::<f32>()) as u64,
        Memory::Default,
    )
    .expect("params allocation");
    let view_buf = GpuAllocation::new(
        &device,
        (VIEW_FLOATS * size_of::<f32>()) as u64,
        Memory::Default,
    )
    .expect("view allocation");
    let out = GpuAllocation::new(
        &device,
        (N * OUT_PPG * size_of::<f32>()) as u64,
        Memory::Default,
    )
    .expect("output allocation");
    write_floats(&params, &params_cpu);
    write_floats(&view_buf, &view_cpu);

    // The virtual module path places the wrapper next to gaussian.slang;
    // the import resolves through it.
    let module_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/shaders/gs/__gs_math_test.slang"
    );
    let cache = device.shader_cache();
    let compiled = cache
        .compile_source(module_path, WRAPPER, "eval", &[], &[])
        .expect("compile the wrapper");
    let module = ShaderModule::from_compiled(&device, &compiled).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    cmd.bind_compute_pipeline(&pipeline);
    push_roots(&cmd, &[params.gpu(), view_buf.gpu(), out.gpu()]);
    cmd.dispatch(1, 1, 1);
    cmd.end().expect("end");
    device.submit_and_wait(&[&cmd]).expect("submit");

    let gpu = read_floats(&out, N * OUT_PPG);
    for (i, g) in gaussians.iter().enumerate() {
        let reference = reference(g, &view);
        for (c, name) in COMPONENTS.iter().enumerate() {
            assert_close(i, name, gpu[i * OUT_PPG + c], reference[c]);
        }
    }
}

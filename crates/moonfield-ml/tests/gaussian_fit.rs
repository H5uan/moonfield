//! Acceptance for the ml training loop on the public RHI API.
//!
//! The rhi `gaussian_fit` spike's problem — 64 2D Gaussians fitted to a
//! 128×128 procedural target — driven through `Trainer` and
//! `TrainingMethod` with the atomic-accumulation backward and the
//! asset-loaded Adam kernel, no backend types in sight. Acceptance is
//! statistical (final/initial loss ratio ≤ 0.2): atomics interleave
//! nondeterministically, so runs are not bit-reproducible — the roadmap's
//! standing decision for training loops.

use std::path::PathBuf;

use moonfield_asset::{AssetServer, Assets};
use moonfield_ml::optimizer::{Adam, AdamParams};
use moonfield_ml::trainer::{Trainer, TrainingMethod};
use moonfield_rhi::{
    BarrierHazard, CommandBuffer, ComputePipeline, Device, GpuAllocation, GpuPtr, Instance, Memory,
    ShaderModule, Stage,
};
use moonfield_shader::{Shader, SlangLoader};

/// Number of Gaussians in the mixture.
const N: usize = 64;
/// Scalars per Gaussian: mean.xy, log_scale.xy, rotation, color.rgb,
/// logit_opacity.
const PPG: usize = 9;
const SCALARS: usize = N * PPG;
/// Square image edge in pixels.
const SIZE: usize = 128;
const PIXELS: usize = SIZE * SIZE;
/// Trainer iterations.
const ITERS: u32 = 600;

/// One Slang source, two compute entries; compiled once per entry point (the
/// shader cache memoizes by source and entry). Parameters are a flat float
/// buffer laid out `[N][9]`; opacity/scale transforms live inside the
/// differentiable function so autodiff covers them. The backward pass
/// accumulates each pixel's contribution straight into the per-Gaussian
/// gradient slots with buffer float atomics — no intermediate
/// per-(pixel, Gaussian) records.
const SLANG_SOURCE: &str = r#"
static const int N = 64;
static const int PPG = 9;
static const int SIZE = 128;

struct Gaussian : IDifferentiable
{
    float2 mean;
    float2 log_scale;
    float rotation;
    float3 color;
    float logit_opacity;
};

[Differentiable]
float3 contrib(no_diff float2 uv, Gaussian g)
{
    float opacity = 1.0 / (1.0 + exp(-g.logit_opacity));
    float2 scale = exp(g.log_scale);
    float c = cos(g.rotation);
    float s = sin(g.rotation);
    float2 d = uv - g.mean;
    // Sigma^-1 = R diag(1/sx^2, 1/sy^2) R^T, evaluated as (R^T d)^2 / s^2.
    float2 rd = float2(c * d.x + s * d.y, -s * d.x + c * d.y);
    float2 inv = 1.0 / (scale * scale);
    float m = rd.x * rd.x * inv.x + rd.y * rd.y * inv.y;
    float alpha = opacity * exp(-0.5 * m);
    return g.color * alpha;
}

Gaussian load_gaussian(Ptr<float, Access.Read> params, int i)
{
    int o = i * PPG;
    Gaussian g;
    g.mean = float2(params[o + 0], params[o + 1]);
    g.log_scale = float2(params[o + 2], params[o + 3]);
    g.rotation = params[o + 4];
    g.color = float3(params[o + 5], params[o + 6], params[o + 7]);
    g.logit_opacity = params[o + 8];
    return g;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void forward(uint3 tid : SV_DispatchThreadID,
             Ptr<float, Access.Read> params,
             Ptr<float, Access.Read> target,
             Ptr<float, Access.ReadWrite> image,
             Ptr<float, Access.ReadWrite> loss)
{
    if (tid.x >= SIZE || tid.y >= SIZE) return;
    uint px = tid.y * SIZE + tid.x;
    float2 uv = float2((float(tid.x) + 0.5) / SIZE, (float(tid.y) + 0.5) / SIZE);
    float3 acc = float3(0.0);
    for (int i = 0; i < N; i++)
        acc += contrib(uv, load_gaussian(params, i));
    image[px * 3 + 0] = acc.x;
    image[px * 3 + 1] = acc.y;
    image[px * 3 + 2] = acc.z;
    float3 err = acc - float3(target[px * 3], target[px * 3 + 1], target[px * 3 + 2]);
    loss[px] = dot(err, err);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void backward(uint3 tid : SV_DispatchThreadID,
              Ptr<float, Access.Read> params,
              Ptr<float, Access.Read> target,
              Ptr<float, Access.Read> image,
              Ptr<float, Access.ReadWrite> grads)
{
    if (tid.x >= SIZE || tid.y >= SIZE) return;
    uint px = tid.y * SIZE + tid.x;
    float2 uv = float2((float(tid.x) + 0.5) / SIZE, (float(tid.y) + 0.5) / SIZE);
    // dL/drendered for the summed squared error.
    float3 dL = 2.0 * (float3(image[px * 3], image[px * 3 + 1], image[px * 3 + 2])
                       - float3(target[px * 3], target[px * 3 + 1], target[px * 3 + 2]));
    for (int i = 0; i < N; i++)
    {
        // Buffer loads are non-differentiable global ops; the gradient flows
        // through the local differential pair.
        Gaussian g = load_gaussian(params, i);
        var dp = diffPair(g, Gaussian());
        bwd_diff(contrib)(uv, dp, dL);
        uint o = i * PPG;
        __atomic_add(grads[o + 0], dp.d.mean.x);
        __atomic_add(grads[o + 1], dp.d.mean.y);
        __atomic_add(grads[o + 2], dp.d.log_scale.x);
        __atomic_add(grads[o + 3], dp.d.log_scale.y);
        __atomic_add(grads[o + 4], dp.d.rotation);
        __atomic_add(grads[o + 5], dp.d.color.x);
        __atomic_add(grads[o + 6], dp.d.color.y);
        __atomic_add(grads[o + 7], dp.d.color.z);
        __atomic_add(grads[o + 8], dp.d.logit_opacity);
    }
}
"#;

/// Deterministic xorshift32 so the init is reproducible across runs/machines.
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

/// The target: a smooth radial gradient plus a warm disc — non-trivial but
/// well within what 64 Gaussians can fit.
fn build_target() -> Vec<f32> {
    let mut target = vec![0.0f32; PIXELS * 3];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = (x as f32 + 0.5) / SIZE as f32;
            let v = (y as f32 + 0.5) / SIZE as f32;
            let d1 = ((u - 0.35).powi(2) + (v - 0.35).powi(2)).sqrt();
            let d2 = ((u - 0.70).powi(2) + (v - 0.65).powi(2)).sqrt();
            let mut rgb = [
                0.30 + 0.40 * (1.0 - (d1 * 2.5).min(1.0)),
                0.25 + 0.30 * v,
                0.35 + 0.30 * u,
            ];
            if d2 < 0.18 {
                rgb = [0.85, 0.55, 0.15];
            }
            let px = (y * SIZE + x) * 3;
            target[px..px + 3].copy_from_slice(&rgb);
        }
    }
    target
}

/// Initial parameters: means on a jittered 8x8 grid over [0,1]^2, small
/// isotropic scales, random colors, opacity 0.5 (logit 0).
fn build_params() -> Vec<f32> {
    let mut rng = Rng(0x1234_5678);
    let mut params = vec![0.0f32; SCALARS];
    for i in 0..N {
        let gx = (i % 8) as f32;
        let gy = (i / 8) as f32;
        let o = i * 9;
        params[o] = (gx + 0.5) / 8.0 + (rng.next() - 0.5) * 0.05;
        params[o + 1] = (gy + 0.5) / 8.0 + (rng.next() - 0.5) * 0.05;
        params[o + 2] = 0.08f32.ln();
        params[o + 3] = 0.08f32.ln();
        params[o + 4] = (rng.next() - 0.5) * 0.5;
        params[o + 5] = 0.2 + 0.6 * rng.next();
        params[o + 6] = 0.2 + 0.6 * rng.next();
        params[o + 7] = 0.2 + 0.6 * rng.next();
        params[o + 8] = 0.0;
    }
    params
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

/// Push a list of root pointers as consecutive u64s, matching the Slang
/// entry parameters in declaration order — the pure-pointer-kernel layout
/// the rhi spike proved; `RootBinder` places are the production shape.
fn push_roots(cmd: &CommandBuffer, ptrs: &[GpuPtr]) {
    let mut bytes = Vec::with_capacity(ptrs.len() * 8);
    for ptr in ptrs {
        bytes.extend_from_slice(&ptr.as_raw().to_le_bytes());
    }
    cmd.push_data(0, &bytes);
}

/// The minimal method: the spike's 2D fit problem as a
/// [`TrainingMethod`], with the atomic-accumulation backward and the
/// asset-loaded Adam kernel.
struct GaussianFit {
    params: GpuAllocation,
    grads: GpuAllocation,
    target: GpuAllocation,
    image: GpuAllocation,
    loss: GpuAllocation,
    forward: ComputePipeline,
    backward: ComputePipeline,
    adam: Adam,
    /// Loss at each reporting point; the acceptance reads the first/last pair.
    loss_history: Vec<f32>,
}

impl GaussianFit {
    fn new(device: &Device, adam_shader: &Shader) -> Self {
        let cache = device.shader_cache();
        let compile = |entry: &str| -> ComputePipeline {
            let shader = cache
                .compile_source("gaussian_fit", SLANG_SOURCE, entry, &[], &[])
                .unwrap_or_else(|e| panic!("Slang compilation of '{entry}' failed: {e}"));
            let module = ShaderModule::from_compiled(device, &shader).expect("shader module");
            ComputePipeline::new(device, &module).expect("compute pipeline")
        };
        let forward = compile("forward");
        let backward = compile("backward");

        let floats = |count: usize| {
            GpuAllocation::new(device, (count * size_of::<f32>()) as u64, Memory::Default)
                .expect("allocation")
        };
        let params = floats(SCALARS);
        let grads = floats(SCALARS);
        let target = floats(PIXELS * 3);
        let image = floats(PIXELS * 3);
        let loss = floats(PIXELS);

        write_floats(&params, &build_params());
        write_floats(&target, &build_target());

        let adam = Adam::new(
            device,
            adam_shader,
            SCALARS,
            AdamParams {
                lr: 0.02,
                ..Default::default()
            },
        )
        .expect("adam");

        Self {
            params,
            grads,
            target,
            image,
            loss,
            forward,
            backward,
            adam,
            loss_history: Vec::new(),
        }
    }
}

impl TrainingMethod for GaussianFit {
    fn record_step(&mut self, cmd: &CommandBuffer, step: u32) {
        // The backward accumulates atomically, so the gradient buffer starts
        // at zero every step. The synchronous loop leaves the GPU idle while
        // recording; the production method clears kernel-side per the roadmap.
        // SAFETY: the grads allocation is host-visible, persistently mapped,
        // and sized SCALARS floats.
        unsafe {
            std::ptr::write_bytes(
                self.grads.host().expect("grads host view").typed::<f32>(),
                0,
                SCALARS,
            );
        }

        cmd.bind_compute_pipeline(&self.forward);
        push_roots(
            cmd,
            &[
                self.params.gpu(),
                self.target.gpu(),
                self.image.gpu(),
                self.loss.gpu(),
            ],
        );
        cmd.dispatch((SIZE / 8) as u32, (SIZE / 8) as u32, 1);
        cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);

        cmd.bind_compute_pipeline(&self.backward);
        push_roots(
            cmd,
            &[
                self.params.gpu(),
                self.target.gpu(),
                self.image.gpu(),
                self.grads.gpu(),
            ],
        );
        cmd.dispatch((SIZE / 8) as u32, (SIZE / 8) as u32, 1);
        cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);

        self.adam.record_step(cmd, &self.params, &self.grads, step);
    }

    fn readback_loss(&mut self) -> f32 {
        // SAFETY: host-visible and sized PIXELS floats; the trainer calls
        // this after `submit_and_wait`, so the GPU is done writing.
        let host = self.loss.host().expect("loss host view").typed::<f32>();
        let total: f64 = (0..PIXELS).map(|i| unsafe { *host.add(i) as f64 }).sum();
        let loss = total as f32;
        self.loss_history.push(loss);
        loss
    }
}

#[test]
fn gaussian_fit_converges_through_trainer() {
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

    let mut server = AssetServer::default();
    server.register_loader(SlangLoader);
    let mut assets = Assets::<Shader>::default();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/shaders/ml/adam.slang");
    let handle = server
        .load(&mut assets, &path)
        .expect("load adam.slang through the asset server");
    let adam_shader = assets.get(&handle).expect("shader asset");

    let mut method = GaussianFit::new(&device, adam_shader);
    let mut trainer = Trainer::new(&device, 50).expect("trainer");
    trainer.run(&mut method, ITERS);

    let history = &method.loss_history;
    assert!(
        history.len() >= 2,
        "expected loss reports at the first and final step"
    );
    for (i, loss) in history.iter().enumerate() {
        assert!(
            loss.is_finite(),
            "loss went non-finite at report {i}: {loss}"
        );
    }
    let initial = history[0] as f64;
    let final_loss = *history.last().unwrap() as f64;
    let ratio = final_loss / initial;
    println!("initial loss = {initial:.4}, final loss = {final_loss:.4}, ratio = {ratio:.4}");
    assert!(
        ratio <= 0.2,
        "training did not converge: final/initial loss ratio {ratio:.3} > 0.2"
    );
}

//! Slang-autodiff spike: fit a 2D Gaussian mixture to a procedural image.
//!
//! Proves the full training stack on the RHI's compute path: one Slang source
//! with `[Differentiable]` code and `bwd_diff` compiles to SPIR-V, dispatches
//! through the bindless descriptor-heap pipeline path, and a hand-written Adam
//! kernel drives the loss down — no CPU-side math in the loop. The test skips
//! on machines without `VK_EXT_descriptor_heap` (see `common`).

use moonfield_rhi::{
    BarrierHazard, CommandBuffer, CommandBufferUsage, CommandPool, Compiler, ComputePipeline,
    Device, GpuAllocation, GpuPtr, Instance, Memory, ShaderModule, Stage,
};
mod common;

/// Number of Gaussians in the mixture.
const N: usize = 64;
/// Scalars per Gaussian: mean.xy, log_scale.xy, rotation, color.rgb,
/// logit_opacity.
const SCALARS: usize = N * 9;
/// Square image edge in pixels.
const SIZE: usize = 128;
const PIXELS: usize = SIZE * SIZE;
/// Adam iterations.
const ITERS: u32 = 600;

/// One Slang source, four compute entries; compiled once per entry point (the
/// compiler memoizes by (source, entry)). Parameters are a flat float buffer
/// laid out `[N][9]`; opacity/scale transforms live inside the differentiable
/// function so autodiff covers them. The backward pass writes one gradient
/// record per (pixel, gaussian) — no atomics — and `reduce` sums them.
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
              Ptr<float, Access.ReadWrite> gradbuf)
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
        uint o = (px * N + i) * PPG;
        gradbuf[o + 0] = dp.d.mean.x;
        gradbuf[o + 1] = dp.d.mean.y;
        gradbuf[o + 2] = dp.d.log_scale.x;
        gradbuf[o + 3] = dp.d.log_scale.y;
        gradbuf[o + 4] = dp.d.rotation;
        gradbuf[o + 5] = dp.d.color.x;
        gradbuf[o + 6] = dp.d.color.y;
        gradbuf[o + 7] = dp.d.color.z;
        gradbuf[o + 8] = dp.d.logit_opacity;
    }
}

[shader("compute")]
[numthreads(64, 1, 1)]
void reduce(uint3 tid : SV_DispatchThreadID,
            Ptr<float, Access.Read> gradbuf,
            Ptr<float, Access.ReadWrite> grads)
{
    uint s = tid.x;
    if (s >= N * PPG) return;
    uint gi = s / PPG;
    uint c = s % PPG;
    float sum = 0.0;
    for (uint px = 0; px < SIZE * SIZE; px++)
        sum += gradbuf[(px * N + gi) * PPG + c];
    grads[s] = sum;
}

[shader("compute")]
[numthreads(64, 1, 1)]
void adam(uint3 tid : SV_DispatchThreadID,
          Ptr<float, Access.ReadWrite> params,
          Ptr<float, Access.Read> grads,
          Ptr<float, Access.ReadWrite> m,
          Ptr<float, Access.ReadWrite> v,
          Ptr<float, Access.Read> meta)
{
    uint s = tid.x;
    if (s >= N * PPG) return;
    float t = meta[0];
    float g = grads[s];
    float b1 = 0.9;
    float b2 = 0.999;
    float lr = 0.02;
    float mi = b1 * m[s] + (1.0 - b1) * g;
    float vi = b2 * v[s] + (1.0 - b2) * g * g;
    m[s] = mi;
    v[s] = vi;
    float mhat = mi / (1.0 - pow(b1, t));
    float vhat = vi / (1.0 - pow(b2, t));
    params[s] -= lr * mhat / (sqrt(vhat) + 1e-8);
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

/// Push a list of root pointers as consecutive u64s, matching the Slang entry
/// parameters in declaration order (8 bytes each, see `set_bindless_root`).
fn push_roots(cmd: &CommandBuffer, ptrs: &[GpuPtr]) {
    let mut bytes = Vec::with_capacity(ptrs.len() * 8);
    for ptr in ptrs {
        bytes.extend_from_slice(&ptr.as_raw().to_le_bytes());
    }
    cmd.push_data(0, &bytes);
}

fn write_floats(alloc: &GpuAllocation, data: &[f32]) {
    let host = alloc.host().expect("allocation must have a host view");
    // SAFETY: the allocation is host-visible, persistently mapped, and sized
    // to hold `data`.
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), host.typed::<f32>(), data.len());
    }
}

fn read_loss(loss: &GpuAllocation) -> f64 {
    let host = loss.host().expect("loss must have a host view");
    let mut sum = 0.0f64;
    for i in 0..PIXELS {
        // SAFETY: host-visible and sized PIXELS floats; read after
        // queue_wait_idle, so the GPU is done writing.
        sum += unsafe { *host.typed::<f32>().add(i) } as f64;
    }
    sum
}

#[test]
fn gaussian_fit_converges() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return;
        }
    };
    if common::skip_if_descriptor_heap_missing(&instance) {
        return;
    }
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };

    // One compilation per entry point; the four pipelines share the source.
    let compiler = Compiler::new().expect("compiler creation");
    let mut pipelines = Vec::new();
    for entry in ["forward", "backward", "reduce", "adam"] {
        let spirv = compiler
            .compile_source_to_spirv("gaussian_fit", SLANG_SOURCE, entry)
            .unwrap_or_else(|e| panic!("Slang compilation of '{entry}' failed: {e}"));
        let module = ShaderModule::from_compiled(&device, &spirv).expect("shader module");
        pipelines.push(ComputePipeline::new(&device, &module).expect("compute pipeline"));
    }
    let mut pipelines = pipelines.into_iter();
    let forward = pipelines.next().unwrap();
    let backward = pipelines.next().unwrap();
    let reduce = pipelines.next().unwrap();
    let adam = pipelines.next().unwrap();

    let params = GpuAllocation::new(&device, (SCALARS * 4) as u64, Memory::Default).unwrap();
    let grads = GpuAllocation::new(&device, (SCALARS * 4) as u64, Memory::Default).unwrap();
    let adam_m = GpuAllocation::new(&device, (SCALARS * 4) as u64, Memory::Default).unwrap();
    let adam_v = GpuAllocation::new(&device, (SCALARS * 4) as u64, Memory::Default).unwrap();
    let target = GpuAllocation::new(&device, (PIXELS * 3 * 4) as u64, Memory::Default).unwrap();
    let image = GpuAllocation::new(&device, (PIXELS * 3 * 4) as u64, Memory::Default).unwrap();
    let loss = GpuAllocation::new(&device, (PIXELS * 4) as u64, Memory::Default).unwrap();
    // Per-(pixel, gaussian) gradient records: ~38 MB.
    let gradbuf =
        GpuAllocation::new(&device, (PIXELS * SCALARS * 4) as u64, Memory::Default).unwrap();
    // Adam step counter (t as float at offset 0); the CPU bumps it per
    // iteration through the persistent mapping.
    let meta = GpuAllocation::new(&device, 16, Memory::Default).unwrap();

    write_floats(&params, &build_params());
    write_floats(&target, &build_target());
    write_floats(&adam_m, &vec![0.0f32; SCALARS]);
    write_floats(&adam_v, &vec![0.0f32; SCALARS]);

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).unwrap();
    let mut cmd = pool.allocate_command_buffer().unwrap();

    let mut initial_loss = None;
    let mut final_loss = f64::NAN;
    for iter in 1..=ITERS {
        write_floats(&meta, &[iter as f32]);

        // One submission per iteration: forward -> backward -> reduce -> adam,
        // ordered by compute-to-compute memory barriers.
        cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT).unwrap();
        cmd.bind_compute_pipeline(&forward);
        push_roots(&cmd, &[params.gpu(), target.gpu(), image.gpu(), loss.gpu()]);
        cmd.dispatch((SIZE / 8) as u32, (SIZE / 8) as u32, 1);
        cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);
        cmd.bind_compute_pipeline(&backward);
        push_roots(
            &cmd,
            &[params.gpu(), target.gpu(), image.gpu(), gradbuf.gpu()],
        );
        cmd.dispatch((SIZE / 8) as u32, (SIZE / 8) as u32, 1);
        cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);
        cmd.bind_compute_pipeline(&reduce);
        push_roots(&cmd, &[gradbuf.gpu(), grads.gpu()]);
        cmd.dispatch(SCALARS.div_ceil(64) as u32, 1, 1);
        cmd.barrier(Stage::COMPUTE, Stage::COMPUTE, BarrierHazard::Memory);
        cmd.bind_compute_pipeline(&adam);
        push_roots(
            &cmd,
            &[
                params.gpu(),
                grads.gpu(),
                adam_m.gpu(),
                adam_v.gpu(),
                meta.gpu(),
            ],
        );
        cmd.dispatch(SCALARS.div_ceil(64) as u32, 1, 1);
        cmd.end().unwrap();

        let commands = [cmd.raw()];
        let submit_info = ash::vk::SubmitInfo::default().command_buffers(&commands);
        unsafe {
            device
                .raw()
                .queue_submit(
                    device.graphics_queue(),
                    &[submit_info],
                    ash::vk::Fence::null(),
                )
                .expect("submit");
            device
                .raw()
                .queue_wait_idle(device.graphics_queue())
                .expect("wait for idle");
        }

        if iter == 1 || iter % 50 == 0 || iter == ITERS {
            let total = read_loss(&loss);
            assert!(total.is_finite(), "loss went non-finite at iter {iter}");
            if iter == 1 {
                initial_loss = Some(total);
            }
            final_loss = total;
            println!("iter {iter}: loss = {total:.4}");
        }
    }

    let initial_loss = initial_loss.unwrap();
    let ratio = final_loss / initial_loss;
    println!("initial loss = {initial_loss:.4}, final loss = {final_loss:.4}, ratio = {ratio:.4}");
    assert!(
        ratio <= 0.2,
        "training did not converge: final/initial loss ratio {ratio:.3} > 0.2"
    );
}

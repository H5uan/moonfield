//! Compute and autodiff tests for the Metal 4 backend.
//!
//! `compute_dispatches` runs a Slang compute kernel (compiled to metallib
//! through the shared compiler) over a buffer through the argument table.
//! `autodiff_fwd_numeric` compiles `[Differentiable]` code with `fwddiff`
//! to metallib and checks the forward derivative against the analytic one —
//! the ml path's compilation target. Both skip gracefully on machines
//! without a Metal 4 device.

use crate::{
    CommandPool, Compiler, ComputePipeline, Device, Instance, Memory, ShaderModule, ShaderTarget,
};

fn metal4_device() -> Option<Device> {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Metal 4 device available ({err})");
            return None;
        }
    };
    Some(Device::new(&instance).expect("device"))
}

const PLUS_ONE: &str = r#"
[shader("compute")]
void plus_one(uint3 tid : SV_DispatchThreadID, Ptr<uint32_t, Access.ReadWrite> out)
{
    out[tid.x] = out[tid.x] + 1u;
}
"#;

const COUNT: usize = 256;

#[test]
fn compute_dispatches() {
    let Some(device) = metal4_device() else {
        return;
    };

    let compiler = Compiler::new().expect("compiler");
    // Control: a handwritten MSL kernel (plain `[[buffer(0)]]` binding)
    // isolates the encoder path from the Slang codegen.
    let control = ShaderModule::from_msl(
        &device,
        r#"
#include <metal_stdlib>
using namespace metal;
kernel void plus_one_msl(device uint* out [[buffer(0)]], uint tid [[thread_position_in_grid]])
{
    out[tid] = out[tid] + 1u;
}
"#,
    )
    .expect("compile control msl");
    let control_pipeline =
        ComputePipeline::new(&device, &control, "plus_one_msl").expect("control pipeline");
    {
        let memory = Memory::new(&device, (COUNT * 4) as u64);
        let values: *mut u32 = memory.host_ptr() as _;
        // SAFETY: shared storage; single-writer frame contract.
        unsafe {
            for i in 0..COUNT {
                *values.add(i) = i as u32;
            }
        }
        let pool = CommandPool::new(&device);
        let mut cmd = pool.allocate();
        cmd.begin_compute();
        cmd.bind_pipeline(&control_pipeline);
        cmd.set_buffer(0, &memory.allocation());
        cmd.dispatch(8, 1, 1);
        cmd.end_compute();
        device.submit_and_wait(cmd).expect("submit control");
        // SAFETY: GPU work has completed.
        assert_eq!(unsafe { *values.add(0) }, 1, "control kernel must run");
    }

    let compiled = compiler
        .compile_source("plus_one", PLUS_ONE, "plus_one", ShaderTarget::MetalLib)
        .expect("compile to metallib");
    let module = ShaderModule::from_compiled(&device, &compiled).expect("load metallib");
    let pipeline = ComputePipeline::new(&device, &module, &compiled.entry).expect("pipeline");

    let memory = Memory::new(&device, (COUNT * 4) as u64);
    let values: *mut u32 = memory.host_ptr() as _;
    // SAFETY: shared storage; single-writer frame contract; COUNT u32s fit.
    unsafe {
        for i in 0..COUNT {
            *values.add(i) = i as u32;
        }
    }

    // Slang's metal codegen packs root parameters into an `EntryPointParams`
    // struct at buffer(0) — the root-blob model the Vulkan backend's
    // RootBinder builds. The blob for a single `Ptr` root is the 8-byte GPU
    // address of the target buffer.
    let root_blob = Memory::new(&device, 8);
    root_blob.copy_host(0, &memory.gpu_ptr().as_raw().to_le_bytes());

    let pool = CommandPool::new(&device);
    let mut cmd = pool.allocate();
    cmd.begin_compute();
    cmd.bind_pipeline(&pipeline);
    cmd.set_buffer(0, &root_blob.allocation());
    cmd.dispatch(8, 1, 1);
    cmd.end_compute();
    device.submit_and_wait(cmd).expect("submit");

    for i in 0..COUNT {
        // SAFETY: same single-writer contract; GPU work has completed.
        let got = unsafe { *values.add(i) };
        assert_eq!(got, i as u32 + 1, "value {i}");
    }
}

const AUTODIFF: &str = r#"
[Differentiable]
float polynomial(float x)
{
    return 2.0 * x * x + 3.0 * x + 1.0;
}

[shader("compute")]
void fwd(uint3 tid : SV_DispatchThreadID, Ptr<float> xs, Ptr<float> ds)
{
    let xp = DifferentialPair<float>(xs[tid.x], 1.0);
    let yp = fwd_diff(polynomial)(xp);
    ds[tid.x] = yp.d;
}
"#;

const PUSH_SCALE: &str = r#"
[shader("compute")]
void scale(uint3 tid : SV_DispatchThreadID, uniform float factor, Ptr<float> data)
{
    data[tid.x] = data[tid.x] * factor;
}
"#;

/// `push_data` snapshots: two pushes with different uniform values, one
/// dispatch each — the second dispatch must read the second value even
/// though both share argument-table slot 0's root blob.
#[test]
fn push_data_snapshots_between_dispatches() {
    let Some(device) = metal4_device() else {
        return;
    };

    let compiler = Compiler::new().expect("compiler");
    let compiled = compiler
        .compile_source("push_scale", PUSH_SCALE, "scale", ShaderTarget::MetalLib)
        .expect("compile to metallib");
    let module = ShaderModule::from_compiled(&device, &compiled).expect("load metallib");
    let pipeline = ComputePipeline::new(&device, &module, &compiled.entry).expect("pipeline");

    const COUNT: usize = 16;
    let data = Memory::new(&device, (COUNT * 4) as u64);
    let values: *mut f32 = data.host_ptr() as _;
    // SAFETY: shared storage; single-writer frame contract.
    unsafe {
        for i in 0..COUNT {
            *values.add(i) = i as f32;
        }
    }

    let pool = CommandPool::new(&device);
    let mut cmd = pool.allocate();
    cmd.begin_compute();
    cmd.bind_pipeline(&pipeline);
    cmd.set_buffer(0, &data.allocation());
    // Root blob: uniform `factor` at offset 0, `Ptr<float> data` at 8.
    fn push_root(cmd: &mut crate::CommandBuffer, factor: f32, data: &Memory) {
        let mut blob = [0u8; 16];
        blob[0..4].copy_from_slice(&factor.to_le_bytes());
        blob[8..16].copy_from_slice(&data.gpu_ptr().as_raw().to_le_bytes());
        cmd.push_data(0, &blob);
    }
    push_root(&mut cmd, 2.0, &data);
    cmd.dispatch(1, 1, 1);
    push_root(&mut cmd, 3.0, &data);
    cmd.dispatch(1, 1, 1);
    cmd.end_compute();
    device.submit_and_wait(cmd).expect("submit");

    for i in 0..COUNT {
        // i × 2 (first push) × 3 (second push).
        let expected = i as f32 * 6.0;
        // SAFETY: GPU work has completed.
        let got = unsafe { *values.add(i) };
        assert!(
            (got - expected).abs() < 1e-4,
            "value {i}: {got}, expected {expected}"
        );
    }
}

#[test]
fn autodiff_fwd_numeric() {
    let Some(device) = metal4_device() else {
        return;
    };

    let compiler = Compiler::new().expect("compiler");
    let compiled = compiler
        .compile_source("autodiff_fwd", AUTODIFF, "fwd", ShaderTarget::MetalLib)
        .expect("compile autodiff to metallib");
    let module = ShaderModule::from_compiled(&device, &compiled).expect("load metallib");
    let pipeline = ComputePipeline::new(&device, &module, &compiled.entry).expect("pipeline");

    const COUNT: usize = 64;
    let xs_memory = Memory::new(&device, (COUNT * 4) as u64);
    let ds_memory = Memory::new(&device, (COUNT * 4) as u64);
    let xs: *mut f32 = xs_memory.host_ptr() as _;
    // SAFETY: shared storage; single-writer frame contract.
    unsafe {
        for i in 0..COUNT {
            *xs.add(i) = i as f32 * 0.25 - 8.0;
        }
    }

    // Root blob: two `Ptr` roots in declaration order — the layout the
    // shared `Reflection::root_parameters` describes (and the Vulkan
    // RootBinder fills by name).
    let root_blob = Memory::new(&device, 16);
    root_blob.copy_host(0, &xs_memory.gpu_ptr().as_raw().to_le_bytes());
    root_blob.copy_host(8, &ds_memory.gpu_ptr().as_raw().to_le_bytes());

    let pool = CommandPool::new(&device);
    let mut cmd = pool.allocate();
    cmd.begin_compute();
    cmd.bind_pipeline(&pipeline);
    cmd.set_buffer(0, &root_blob.allocation());
    cmd.dispatch(2, 1, 1);
    cmd.end_compute();
    device.submit_and_wait(cmd).expect("submit");

    let ds: *const f32 = ds_memory.host_ptr() as _;
    for i in 0..COUNT {
        // d/dx (2x² + 3x + 1) = 4x + 3
        let x = i as f32 * 0.25 - 8.0;
        let expected = 4.0 * x + 3.0;
        // SAFETY: GPU work has completed.
        let got = unsafe { *ds.add(i) };
        assert!(
            (got - expected).abs() < 1e-4,
            "d/dx at x={x}: {got}, expected {expected}"
        );
    }
}

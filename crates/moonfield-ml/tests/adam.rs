//! Adam kernel verification: two steps against hand-computed values.
//!
//! Four parameters start at 1.0 with zeroed moments, and the gradients
//! differ between the steps (0.5, then 0.25): under a constant gradient the
//! bias-corrected ratio m̂/√v̂ collapses to g/|g|, so a re-zeroed moment
//! buffer would pass — differing gradients make the mistake visible. With
//! lr = 0.1, β1 = 0.9, β2 = 0.999, ε = 1e-8 the hand-computed parameters
//! are 0.9 after step 1 and 0.806770 after step 2.

use std::path::PathBuf;

use moonfield_asset::{AssetServer, Assets};
use moonfield_ml::optimizer::{Adam, AdamParams};
use moonfield_rhi::{
    CommandBuffer, CommandBufferUsage, CommandPool, Device, GpuAllocation, Instance, Memory,
};
use moonfield_shader::{Shader, SlangLoader};

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
    // SAFETY: host-visible and sized `len` floats; callers read after
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

/// Record one Adam step, submit, and wait — the synchronous loop's body.
fn run_step(
    device: &Device,
    adam: &Adam,
    cmd: &mut CommandBuffer,
    params: &GpuAllocation,
    grads: &GpuAllocation,
    step: u32,
) {
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    adam.record_step(cmd, params, grads, step);
    cmd.end().expect("end");
    device.submit_and_wait(&[cmd]).expect("submit");
}

#[test]
fn two_steps_match_hand_computed_adam() {
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
    let shader = assets.get(&handle).expect("shader asset");

    const PARAMS: usize = 4;
    let adam = Adam::new(
        &device,
        shader,
        PARAMS,
        AdamParams {
            lr: 0.1,
            ..Default::default()
        },
    )
    .expect("adam");

    let params = GpuAllocation::new(&device, (PARAMS * size_of::<f32>()) as u64, Memory::Default)
        .expect("params allocation");
    let grads = GpuAllocation::new(&device, (PARAMS * size_of::<f32>()) as u64, Memory::Default)
        .expect("grads allocation");
    write_floats(&params, &[1.0; PARAMS]);

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");

    // Step 1 (g = 0.5): m̂ = 0.5, v̂ = 0.25, update = 0.1 → 0.9.
    write_floats(&grads, &[0.5; PARAMS]);
    run_step(&device, &adam, &mut cmd, &params, &grads, 1);
    for (i, value) in read_floats(&params, PARAMS).into_iter().enumerate() {
        assert!(
            (value - 0.9).abs() < 1e-3,
            "param[{i}] = {value} after step 1, expected 0.9"
        );
    }

    // Step 2 (g = 0.25): m̂ = 0.368421, v̂ = 0.156203, update = 0.093230.
    write_floats(&grads, &[0.25; PARAMS]);
    run_step(&device, &adam, &mut cmd, &params, &grads, 2);
    for (i, value) in read_floats(&params, PARAMS).into_iter().enumerate() {
        assert!(
            (value - 0.806770).abs() < 1e-3,
            "param[{i}] = {value} after step 2, expected 0.806770 \
             (re-zeroed moments give 0.8, a stuck step counter gives 0.774733)"
        );
    }
}

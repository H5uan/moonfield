//! Command and synchronization contract tests against the facade's public API.
//!
//! `submit_frame`/`Timeline`, the pooled command-buffer lifecycle, and the
//! dispatch/barrier contract (`test_dispatch_barrier_readback`) are pinned
//! verbatim: one spelling — `bind_compute_pipeline` + `push_data` +
//! `dispatch`, two-argument `ComputePipeline::new` — records a compute pass,
//! and the barrier alone orders the dispatch against the readback copy
//! (the vocabulary carries no pass concept).

mod common;

use moonfield_rhi::{
    CommandBufferUsage, ComputePipeline, GpuAllocation, GpuPtr, MemoryResidency, ShaderModule,
    Stage,
};

/// Root data for the kernel below: two 64-bit device addresses (input at
/// offset 0, output at offset 8), pushed as one blob.
fn root_bytes(input: GpuPtr, output: GpuPtr) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&input.as_raw().to_le_bytes());
    bytes[8..].copy_from_slice(&output.as_raw().to_le_bytes());
    bytes
}

/// `+1` kernel: out[tid] = in[tid] + 1. Root data is two 64-bit addresses
/// (input @ offset 0, output @ offset 8) pushed as one push-constant blob.
const PLUS_ONE_KERNEL: &str = r#"
[shader("compute")]
[numthreads(8, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID,
          Ptr<uint32_t, Access.Read> input,
          Ptr<uint32_t, Access.ReadWrite> output)
{
    output[tid.x] = input[tid.x] + 1;
}
"#;

/// Two `+1` dispatches separated by a compute→compute barrier: the first
/// writes input→output, the second reads and rewrites output in place, so it
/// must observe the first dispatch's writes (write-after-read through a
/// stage-only barrier). Expected readback: output[i] = input[i] + 2.
#[test]
fn test_dispatch_barrier_readback() {
    let Some(device) = common::headless_device() else {
        return;
    };

    const COUNT: usize = 8;
    let cache = device.shader_cache();
    let compiled = cache
        .compile_source(
            "contract_plus_one",
            PLUS_ONE_KERNEL,
            "main",
            moonfield_rhi::shader_capabilities(),
            &[],
        )
        .expect("kernel compilation");
    let module = ShaderModule::from_compiled(&device, &compiled).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    let input = GpuAllocation::new(&device, (COUNT * 4) as u64, MemoryResidency::CpuWriteOnly)
        .expect("input allocation");
    let output = GpuAllocation::new(&device, (COUNT * 4) as u64, MemoryResidency::CpuWriteOnly)
        .expect("output allocation");

    // CPU writes the input through the mapped host pointer.
    let input_host = input.host().expect("input must have a host view");
    // SAFETY: the allocation is COUNT u32s, host-visible, and owned by this
    // test — no GPU or CPU aliasing of the written range before submission.
    unsafe {
        for i in 0..COUNT {
            *input_host.typed::<u32>().add(i) = i as u32;
        }
    }

    let cmd = device
        .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("command buffer");
    cmd.bind_compute_pipeline(&pipeline);
    cmd.push_data(0, &root_bytes(input.gpu_ptr(), output.gpu_ptr()));
    cmd.dispatch(1, 1, 1);
    // The contract under test: compute writes become visible to later compute
    // reads across a stage-only barrier (no resource list — the shaders touch
    // memory through pointers).
    cmd.barrier(Stage::COMPUTE, Stage::COMPUTE);
    cmd.push_data(0, &root_bytes(output.gpu_ptr(), output.gpu_ptr()));
    cmd.dispatch(1, 1, 1);
    device.submit_and_wait(&[&cmd]).expect("submit");

    let mut readback = [0u8; COUNT * 4];
    output.read_bytes(&mut readback).expect("readback");
    for (i, chunk) in readback.as_chunks::<4>().0.iter().enumerate() {
        let value = u32::from_le_bytes(*chunk);
        assert_eq!(value, i as u32 + 2, "output[{i}] = {value}");
    }
}

/// Two empty frame submissions must return strictly increasing frame numbers
/// (exactly +1 — the number is spent per submission), and the returned token
/// must be a valid device-timeline point: `wait` on it succeeds and
/// `completed` reports it reached.
#[test]
fn test_submit_frame_returns_monotonic_tokens() {
    let Some(device) = common::headless_device() else {
        return;
    };

    let first = device
        .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("first command buffer");
    let second = device
        .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("second command buffer");

    let first_token = device.submit_frame(&first, &[]).expect("first frame");
    let second_token = device.submit_frame(&second, &[]).expect("second frame");
    assert_eq!(
        second_token,
        first_token + 1,
        "frame tokens must increase by exactly one per submission"
    );

    let timeline = device.timeline();
    timeline.wait(second_token).expect("timeline wait");
    assert!(
        timeline.completed(second_token),
        "the timeline must report a waited frame token as completed"
    );
}

/// Pooled command buffers are single-recording, single-submission objects:
/// each round takes a fresh buffer from the device, records into it, submits
/// it, and drops it (Drop returns it to the pool). Repeated rounds must not
/// exhaust or corrupt the pool.
#[test]
fn test_command_buffer_recording_and_reuse() {
    let Some(device) = common::headless_device() else {
        return;
    };

    for round in 0..4 {
        let cmd = device
            .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
            .expect("command buffer");
        // A stage-only barrier is the minimal recordable command that needs
        // no resources; `submit_and_wait` ends the buffer idempotently.
        cmd.barrier(Stage::COMPUTE, Stage::COMPUTE);
        device
            .submit_and_wait(&[&cmd])
            .unwrap_or_else(|err| panic!("round {round}: {err}"));
    }
}

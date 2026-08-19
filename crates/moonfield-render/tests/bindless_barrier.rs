//! Headless smoke test for the bindless stage barrier.
//!
//! Dispatch A writes a fixed value through its output root pointer, a
//! `barrier(COMPUTE, COMPUTE)` orders the writes, and dispatch B reads that
//! buffer and writes 1 to a second buffer only if it observes the expected
//! value. If the barrier were missing, B could read stale memory; with it,
//! the payload propagates.

use moonfield_render::vulkan::bindless::{ComputePipeline, GpuAllocation, Memory, Stage};
use moonfield_render::{CommandPool, Compiler, Device, Instance, ShaderModule};

/// Dispatch A: write all ones into `payload`.
const WRITE_KERNEL: &str = r#"
[shader("compute")]
[numthreads(8, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID,
          Ptr<uint32_t, Access.ReadWrite> payload)
{
    payload[tid.x] = 42;
}
"#;

/// Dispatch B: read `payload`; each thread checks its own element and
/// atomically clears `result[0]` if it sees anything other than 42. The
/// CPU pre-fills `result[0] = 1`, so the final value is 1 only when every
/// thread observed 42.
const CHECK_KERNEL: &str = r#"
[shader("compute")]
[numthreads(8, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID,
          Ptr<uint32_t, Access.Read> payload,
          Ptr<uint32_t, Access.ReadWrite> result)
{
    if (payload[tid.x] != 42) {
        InterlockedMin(result[0], 0);
    }
}
"#;

#[test]
fn bindless_barrier_orders_dispatch() {
    // CI runners without a GPU/Vulkan driver (Windows, macOS) skip this test;
    // Linux CI runs it against lavapipe (Mesa software Vulkan).
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

    let compiler = Compiler::new().expect("compiler creation");
    let write_spirv = compiler
        .compile_source_to_spirv("write", WRITE_KERNEL, "main")
        .expect("write kernel compilation");
    let check_spirv = compiler
        .compile_source_to_spirv("check", CHECK_KERNEL, "main")
        .expect("check kernel compilation");
    let write_module = ShaderModule::from_spirv(&device, &write_spirv).expect("write module");
    let check_module = ShaderModule::from_spirv(&device, &check_spirv).expect("check module");
    let write_pipeline = ComputePipeline::new(&device, &write_module).expect("write pipeline");
    let check_pipeline = ComputePipeline::new(&device, &check_module).expect("check pipeline");

    let payload = GpuAllocation::new(&device, 64, Memory::Default).expect("payload allocation");
    let result = GpuAllocation::new(&device, 64, Memory::Default).expect("result allocation");

    // CPU pre-fills the pass marker; any thread seeing a stale payload
    // clears it atomically.
    let result_host = result.host().expect("result must have a host view");
    unsafe {
        *result_host.typed::<u32>() = 1;
    }

    let pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("command pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .expect("begin");

    // Dispatch A: write 42 into every payload slot.
    cmd.bind_compute_pipeline(write_pipeline.raw());
    cmd.set_bindless_root(write_pipeline.layout(), payload.gpu(), payload.gpu());
    cmd.dispatch(1, 1, 1);

    // The barrier is the point under test: it must make A's writes visible
    // to B without any resource list.
    cmd.barrier(Stage::COMPUTE, Stage::COMPUTE);

    // Dispatch B: read payload, pass an all-42 check.
    cmd.bind_compute_pipeline(check_pipeline.raw());
    cmd.set_bindless_root(check_pipeline.layout(), payload.gpu(), result.gpu());
    cmd.dispatch(1, 1, 1);

    cmd.end().expect("end");

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

    // result[0] == 1 only if every payload element was 42 after the barrier.
    let result_host = result.host().expect("result must have a host view");
    let all_observed = unsafe { *result_host.typed::<u32>() };
    assert_eq!(
        all_observed, 1,
        "barrier did not make dispatch A visible to B"
    );
}

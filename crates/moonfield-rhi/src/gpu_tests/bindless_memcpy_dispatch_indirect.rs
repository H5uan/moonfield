//! Headless smoke tests for the bindless copy and indirect-dispatch paths.
//!
//! Verifies two command-level operations on `GpuAllocation`s:
//! 1. `cmd_memcpy` copies GPU-side data from a source allocation to a
//!    destination allocation through `vkCmdCopyBuffer2` (sync2).
//! 2. `dispatch_indirect` launches a compute kernel whose workgroup counts
//!    are read from a GPU-memory `DispatchIndirectArgs` struct.

use super::common;
use crate::indirect::DispatchIndirectArgs;
use crate::{
    BarrierHazard, CommandBufferUsage, CommandPool, Compiler, ComputePipeline, Device,
    GpuAllocation, Instance, Memory, ShaderModule, Stage,
};
use std::sync::Mutex;

/// Serializes the tests in this binary. Each test creates its own Vulkan
/// instance, device, and allocator; doing so concurrently on one GPU
/// access-violates on some Windows drivers, and the crate confines Vulkan
/// objects to a single thread by rule.
static DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// `+1` kernel: out[tid] = in[tid] + 1. Root data is two 64-bit addresses
/// (input @ offset 0, output @ offset 8) pushed as one push-constant struct.
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

#[test]
fn bindless_memcpy_roundtrip() {
    let _guard = DEVICE_LOCK.lock().unwrap();
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

    const N: usize = 8;
    const SIZE: u64 = (N * 4) as u64;
    let src = GpuAllocation::new(&device, SIZE, Memory::Default).expect("src allocation");
    let dst = GpuAllocation::new(&device, SIZE, Memory::Default).expect("dst allocation");

    let src_host = src.host().expect("src host view");
    for i in 0..N {
        unsafe {
            *src_host.typed::<u32>().add(i) = i as u32;
        }
    }

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("cmd");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    cmd.cmd_memcpy(&dst, &src, SIZE);
    // Make the copied data visible after the copy (transfer -> all stages).
    cmd.barrier(Stage::TRANSFER, Stage::ALL, BarrierHazard::Memory);
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
            .expect("wait");
    }

    let dst_host = dst.host().expect("dst host view");
    for i in 0..N {
        let value = unsafe { *dst_host.typed::<u32>().add(i) };
        assert_eq!(value, i as u32, "dst[{i}] = {value}");
    }
}

#[test]
fn bindless_dispatch_indirect_roundtrip() {
    let _guard = DEVICE_LOCK.lock().unwrap();
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

    let compiler = Compiler::new().expect("compiler");
    let spirv = compiler
        .compile_source_to_spirv("plus_one", PLUS_ONE_KERNEL, "main")
        .expect("kernel compilation");
    let module = ShaderModule::from_compiled(&device, &spirv).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    let input = GpuAllocation::new(&device, 64, Memory::Default).expect("input allocation");
    let output = GpuAllocation::new(&device, 64, Memory::Default).expect("output allocation");
    let args = GpuAllocation::new(&device, 32, Memory::Default).expect("args allocation");

    let input_host = input.host().expect("input host view");
    for i in 0..8u32 {
        unsafe {
            *input_host.typed::<u32>().add(i as usize) = i;
        }
    }

    // Write the indirect arguments: one workgroup (8 threads).
    let args_host = args.host().expect("args host view");
    let args_val = DispatchIndirectArgs { x: 1, y: 1, z: 1 };
    unsafe {
        *args_host.typed::<DispatchIndirectArgs>() = args_val;
    }

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("cmd");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    cmd.bind_compute_pipeline(&pipeline);
    cmd.set_bindless_root(input.gpu(), output.gpu());
    cmd.dispatch_indirect(&args);
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
            .expect("wait");
    }

    let output_host = output.host().expect("output host view");
    for i in 0..8u32 {
        let value = unsafe { *output_host.typed::<u32>().add(i as usize) };
        assert_eq!(value, i + 1, "output[{i}] = {value}");
    }
}

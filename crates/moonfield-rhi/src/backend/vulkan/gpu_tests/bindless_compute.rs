//! Headless smoke test for the bindless compute dispatch path.
//!
//! Verifies the full CPU→GPU→CPU round trip over bindless pointers: the CPU
//! writes input through a mapped host pointer, a Slang compute kernel reads it
//! through a buffer device address root pointer, writes output through a
//! second root pointer, and the CPU reads the result back.

use super::common;
use crate::{
    CommandBufferUsage, Compiler, ComputePipeline, Device, GpuAllocation, Instance,
    MemoryResidency, ShaderModule, ShaderTarget,
};

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
fn bindless_compute_roundtrip() {
    // CI runners without the engine's required `VK_EXT_descriptor_heap`
    // (lavapipe, most drivers) skip this test.
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

    let compiler = Compiler::new().expect("compiler creation");
    let spirv = compiler
        .compile_source("plus_one", PLUS_ONE_KERNEL, "main", ShaderTarget::Spirv)
        .expect("kernel compilation");
    let module = ShaderModule::from_compiled(&device, &spirv).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    let input =
        GpuAllocation::new(&device, 64, MemoryResidency::CpuWriteOnly).expect("input allocation");
    let output =
        GpuAllocation::new(&device, 64, MemoryResidency::CpuWriteOnly).expect("output allocation");

    // CPU writes the input through the mapped host pointer.
    let input_host = input.host().expect("input must have a host view");
    for i in 0..8u32 {
        unsafe {
            *input_host.typed::<u32>().add(i as usize) = i;
        }
    }

    // Record: bind pipeline, push the two root addresses, dispatch one
    // workgroup of 8 threads.
    let cmd = device
        .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("command buffer");
    cmd.bind_compute_pipeline(&pipeline);
    cmd.set_bindless_root(input.gpu_ptr(), output.gpu_ptr());
    cmd.dispatch(1, 1, 1);

    // Submit and synchronize (blocking wait is fine for a smoke test).
    device.submit_and_wait(&[&cmd]).expect("submit");

    // Read back: output[i] must equal input[i] + 1.
    let output_host = output.host().expect("output must have a host view");
    for i in 0..8u32 {
        let value = unsafe { *output_host.typed::<u32>().add(i as usize) };
        assert_eq!(value, i + 1, "output[{i}] = {value}, expected {}", i + 1);
    }
}

//! Buffer float32 atomic-add probe: `VK_EXT_shader_atomic_float` on the RHI
//! compute path.
//!
//! Slang exposes float atomics through the `__atomic_add` intrinsic — the
//! GLSL-compat `atomicAdd` is declared for GLSL source compiles only and is
//! not visible to a Slang→SPIR-V compile. The intrinsic lowers to
//! `OpAtomicFAddEXT` with `SPV_EXT_shader_atomic_float_add` declared
//! automatically. The test skips on machines whose driver lacks the
//! `shaderBufferFloat32AtomicAdd` feature bit (see [`common`]).

use super::common;
use crate::{
    CommandBufferUsage, CommandPool, Compiler, ComputePipeline, Device, GpuAllocation, Instance,
    Memory, ShaderModule,
};

/// 256 threads: slot 0 gains 1.0 per thread, slot 1 gains `float(tid.x)`.
/// Both sums are integers below 2^24, so every partial sum is exact in f32
/// regardless of the atomic interleaving order — the results are deterministic
/// even though atomics have no ordering.
const SLANG_SOURCE: &str = r#"
[shader("compute")]
[numthreads(256, 1, 1)]
void float_atomic_add(uint3 tid : SV_DispatchThreadID,
                      Ptr<float, Access.ReadWrite> out_buf)
{
    __atomic_add(out_buf[0], 1.0);
    __atomic_add(out_buf[1], float(tid.x));
}
"#;

#[test]
fn atomic_add_sums_correctly() {
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
    if !device.buffer_float32_atomic_add() {
        eprintln!("skipping: shaderBufferFloat32AtomicAdd is not supported by this driver");
        return;
    }

    let compiler = Compiler::new().expect("compiler creation");
    let spirv = compiler
        .compile_source_to_spirv("float_atomics", SLANG_SOURCE, "float_atomic_add")
        .unwrap_or_else(|e| panic!("Slang compilation failed: {e}"));
    let module = ShaderModule::from_compiled(&device, &spirv).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    // Two f32 result slots, zeroed through the persistent host mapping.
    let sums = GpuAllocation::new(&device, 8, Memory::Default).unwrap();
    let host = sums.host().expect("allocation must have a host view");
    // SAFETY: the allocation is host-visible, persistently mapped, and sized
    // for two floats.
    unsafe {
        std::ptr::write_bytes(host.typed::<f32>(), 0, 2);
    }

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).unwrap();
    let mut cmd = pool.allocate_command_buffer().unwrap();
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT).unwrap();
    cmd.bind_compute_pipeline(&pipeline);
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&sums.gpu().as_raw().to_le_bytes());
    cmd.push_data(0, &bytes);
    cmd.dispatch(1, 1, 1);
    cmd.end().unwrap();
    device.submit_and_wait(&[&cmd]).expect("submit");

    // SAFETY: host-visible and sized two floats; read after `submit_and_wait`,
    // so the GPU is done writing.
    let results = unsafe { std::slice::from_raw_parts(host.typed::<f32>(), 2) };
    // 256 × 1.0 and Σ 0..=255.
    assert_eq!(results[0], 256.0);
    assert_eq!(results[1], 32640.0);
}

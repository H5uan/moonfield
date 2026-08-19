//! Headless smoke test for the bindless compute dispatch path.
//!
//! Verifies the full CPU→GPU→CPU round trip over bindless pointers: the CPU
//! writes input through a mapped host pointer, a Slang compute kernel reads it
//! through a buffer device address root pointer, writes output through a
//! second root pointer, and the CPU reads the result back.

use moonfield_render::vulkan::bindless::{ComputePipeline, GpuAllocation, Memory};
use moonfield_render::{CommandPool, Compiler, Device, Instance, ShaderModule};

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
    let spirv = compiler
        .compile_source_to_spirv("plus_one", PLUS_ONE_KERNEL, "main")
        .expect("kernel compilation");
    let module = ShaderModule::from_spirv(&device, &spirv).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    let input = GpuAllocation::new(&device, 64, Memory::Default).expect("input allocation");
    let output = GpuAllocation::new(&device, 64, Memory::Default).expect("output allocation");

    // CPU writes the input through the mapped host pointer.
    let input_host = input.host().expect("input must have a host view");
    for i in 0..8u32 {
        unsafe {
            *input_host.typed::<u32>().add(i as usize) = i;
        }
    }

    // Record: bind pipeline, push the two root addresses, dispatch one
    // workgroup of 8 threads.
    let pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("command pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(ash::vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .expect("begin");
    cmd.bind_compute_pipeline(pipeline.raw());
    cmd.set_bindless_root(pipeline.layout(), input.gpu(), output.gpu());
    cmd.dispatch(1, 1, 1);
    cmd.end().expect("end");

    // Submit and synchronize (blocking wait is fine for a smoke test).
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

    // Read back: output[i] must equal input[i] + 1.
    let output_host = output.host().expect("output must have a host view");
    for i in 0..8u32 {
        let value = unsafe { *output_host.typed::<u32>().add(i as usize) };
        assert_eq!(value, i + 1, "output[{i}] = {value}, expected {}", i + 1);
    }
}

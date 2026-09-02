//! Headless tests for [`CommandBuffer::push_data`] — the descriptor-heap
//! extension's push-constant storage class.
//!
//! `push_data_records_cleanly` covers the host-side recording path;
//! `push_data_feeds_root_pointers` covers the GPU side: a compute kernel's
//! root pointers delivered through `vkCmdPushDataEXT` (instead of
//! `vkCmdPushConstants`) must land in the shader's push-constant block.

mod common;

use moonfield_rhi::vulkan::bindless::{ComputePipeline, GpuAllocation, Memory};
use moonfield_rhi::{CommandBufferUsage, CommandPool, Compiler, Device, Instance, ShaderModule};

#[test]
fn push_data_records_cleanly() {
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

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");

    // Two updates at distinct offsets, as the extension's push data bank is
    // offset-addressed like push constants.
    cmd.push_data(0, &[0u8; 64]);
    cmd.push_data(64, &[1u8; 64]);

    cmd.end().expect("end");
    // Not submitted: recording without validation errors is the contract.
}

/// `+1` kernel, identical to the bindless_compute roundtrip: root data is two
/// 64-bit addresses (input @ offset 0, output @ offset 8) — but here they
/// arrive through `vkCmdPushDataEXT` instead of `vkCmdPushConstants`.
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
fn push_data_feeds_root_pointers() {
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
        .compile_source_to_spirv("plus_one_push_data", PLUS_ONE_KERNEL, "main")
        .expect("kernel compilation");
    let module = ShaderModule::from_compiled(&device, &spirv).expect("shader module");
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

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    cmd.bind_compute_pipeline(pipeline.raw());
    // Push data and push constants alias the same bank, last setter wins —
    // the kernel must see exactly these two addresses, so nothing but
    // push_data may touch root state after this call.
    let root: [u64; 2] = [input.gpu().as_raw(), output.gpu().as_raw()];
    let root_bytes = unsafe {
        std::slice::from_raw_parts(root.as_ptr() as *const u8, std::mem::size_of_val(&root))
    };
    cmd.push_data(0, root_bytes);
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

    // Read back: output[i] must equal input[i] + 1.
    let output_host = output.host().expect("output must have a host view");
    for i in 0..8u32 {
        let value = unsafe { *output_host.typed::<u32>().add(i as usize) };
        assert_eq!(value, i + 1, "output[{i}] = {value}, expected {}", i + 1);
    }
}

//! Headless tests for [`CommandBuffer::push_data`] — the descriptor-heap
//! extension's push-constant storage class.
//!
//! `push_data_records_cleanly` covers the host-side recording path;
//! `push_data_feeds_root_pointers` covers the GPU side: a compute kernel's
//! root pointers delivered through `vkCmdPushDataEXT` (instead of
//! `vkCmdPushConstants`) must land in the shader's push-constant block.

mod common;

use moonfield_rhi::{
    CommandBufferUsage, CommandPool, Compiler, ComputePipeline, Device, GpuAllocation, Instance,
    Memory, RootBinder, ShaderModule,
};

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
    cmd.bind_compute_pipeline(&pipeline);
    // Push constants rely on set layout state and are incompatible with
    // descriptor-heap pipelines; push data replaces them. The kernel must
    // see exactly these two addresses, so nothing but push_data may touch
    // root state on this command buffer.
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

/// `scale` kernel: two pointer roots and one inline uniform root, so a
/// single dispatch reads three push-data ranges written by separate calls.
const SCALE_KERNEL: &str = r#"
[shader("compute")]
[numthreads(8, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID,
          Ptr<uint32_t, Access.Read> input,
          uniform uint32_t scale,
          Ptr<uint32_t, Access.ReadWrite> output)
{
    output[tid.x] = input[tid.x] * scale;
}
"#;

/// The push-data bank is offset-addressed and bytes outside each written
/// range persist: three `push_data` calls at their reflected offsets — the
/// uniform written last — must all be read together by one dispatch. This
/// is the static-prefix pattern the egui pass records with.
#[test]
fn push_data_ranges_persist_across_writes() {
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
    let reflection = compiler
        .compile_source_to_reflection("scale_kernel", SCALE_KERNEL, "main")
        .expect("kernel reflection");
    let binder = RootBinder::new(&reflection, "main").expect("root binder");
    let input_place = binder.pointer_param("input").expect("input place");
    let scale_place = binder.uniform_param("scale").expect("scale place");
    let output_place = binder.pointer_param("output").expect("output place");
    drop(reflection);

    let spirv = compiler
        .compile_source_to_spirv("scale_kernel", SCALE_KERNEL, "main")
        .expect("kernel compilation");
    let module = ShaderModule::from_compiled(&device, &spirv).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    let input = GpuAllocation::new(&device, 64, Memory::Default).expect("input allocation");
    let output = GpuAllocation::new(&device, 64, Memory::Default).expect("output allocation");

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
    cmd.bind_compute_pipeline(&pipeline);
    // The pointers first, the uniform last: the earlier ranges must survive
    // the later writes.
    cmd.push_data(
        input_place.offset as u32,
        &input_place
            .pointer_bytes(input.gpu().as_raw())
            .expect("input bytes"),
    );
    cmd.push_data(
        output_place.offset as u32,
        &output_place
            .pointer_bytes(output.gpu().as_raw())
            .expect("output bytes"),
    );
    let scale: u32 = 7;
    cmd.push_data(scale_place.offset as u32, &scale.to_le_bytes());
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

    // Read back: output[i] must equal input[i] * 7 — all three ranges were
    // visible to the single dispatch.
    let output_host = output.host().expect("output must have a host view");
    for i in 0..8u32 {
        let value = unsafe { *output_host.typed::<u32>().add(i as usize) };
        assert_eq!(value, i * 7, "output[{i}] = {value}, expected {}", i * 7);
    }
}

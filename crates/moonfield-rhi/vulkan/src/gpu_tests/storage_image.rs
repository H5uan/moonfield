//! RGBA16F storage-image round trip — the gaussian-splatting intermediate's
//! prerequisite probe.
//!
//! A compute kernel writes the image through an `RWTexture2D` storage-image
//! heap slot, a `barrier(COMPUTE, SHADER_WRITE, COMPUTE, …)` orders the
//! write, and a
//! second dispatch samples the *same image* through its sampled-image slot
//! into a readback buffer. Passing proves all three untested links at once:
//! `STORAGE_IMAGE` descriptors work in the descriptor heap, Slang compiles
//! heap-indexed `RWTexture2D` (with the `spvDescriptorHeapEXT` capability)
//! to working SPIR-V, and the T1000 supports RGBA16F optimal-tiling storage
//! writes.

use super::common;
use crate::{
    Access, CommandBufferUsage, CommandPool, Compiler, ComputePipeline, Device, Format,
    FrameUploader, GpuAllocation, Instance, Memory, ShaderModule, Stage, Texture,
    UPLOAD_ARENA_SIZE,
};

const WIDTH: u32 = 32;
const HEIGHT: u32 = 32;

/// The write kernel: each thread stores its coordinates through the
/// storage-image heap slot (`{0}`).
const WRITE_KERNEL: &str = r#"
[shader("compute")]
[numthreads(8, 8, 1)]
void main(uint3 tid : SV_DispatchThreadID)
{
    RWTexture2D<float4> img = ResourceDescriptorHeap[NonUniformResourceIndex({0})];
    img[tid.xy] = float4(float(tid.x), float(tid.y), 0.5, 1.0);
}
"#;

/// The check kernel: each thread loads its texel through the sampled-image
/// heap slot (`{1}`) and stores it into the readback buffer.
const CHECK_KERNEL: &str = r#"
[shader("compute")]
[numthreads(8, 8, 1)]
void main(uint3 tid : SV_DispatchThreadID,
          Ptr<float4, Access.ReadWrite> output)
{
    Texture2D tex = ResourceDescriptorHeap[NonUniformResourceIndex({1})];
    output[tid.y * 32u + tid.x] = tex.Load(int3(int2(tid.xy), 0));
}
"#;

#[test]
fn rgba16f_storage_image_roundtrip() {
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

    let mut uploader = match FrameUploader::new(&device, UPLOAD_ARENA_SIZE) {
        Ok(uploader) => uploader,
        Err(err) => {
            eprintln!("skipping: frame uploader creation failed ({err})");
            return;
        }
    };
    let image = Texture::storage_image(
        &device,
        &mut uploader,
        WIDTH,
        HEIGHT,
        Format::R16G16B16A16Sfloat,
    )
    .expect("RGBA16F storage image creation");
    let storage_slot = image.storage_handle().expect("storage slot").0;
    let sampled_slot = image.handle().expect("sampled slot").0;
    // The transition recorded at creation must precede the first dispatch.
    uploader.end_frame().expect("flush image transition");

    let compiler = Compiler::new().expect("compiler creation");
    let write_source = WRITE_KERNEL.replace("{0}", &storage_slot.to_string());
    let check_source = CHECK_KERNEL.replace("{1}", &sampled_slot.to_string());
    let write_spirv = compiler
        .compile_source_to_spirv_with_capabilities(
            "write",
            &write_source,
            "main",
            &["spvDescriptorHeapEXT"],
        )
        .expect("write kernel compilation");
    let check_spirv = compiler
        .compile_source_to_spirv_with_capabilities(
            "check",
            &check_source,
            "main",
            &["spvDescriptorHeapEXT"],
        )
        .expect("check kernel compilation");
    let write_module = ShaderModule::from_compiled(&device, &write_spirv).expect("write module");
    let check_module = ShaderModule::from_compiled(&device, &check_spirv).expect("check module");
    let write_pipeline = ComputePipeline::new(&device, &write_module).expect("write pipeline");
    let check_pipeline = ComputePipeline::new(&device, &check_module).expect("check pipeline");

    let result = GpuAllocation::new(
        &device,
        (WIDTH * HEIGHT) as u64 * std::mem::size_of::<[f32; 4]>() as u64,
        Memory::Readback,
    )
    .expect("result allocation");

    let heap = device.descriptor_heap();
    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    // The heaps must be bound before the shader's ResourceDescriptorHeap
    // accesses resolve to anything.
    heap.cmd_bind(&cmd).expect("bind heaps");

    cmd.bind_compute_pipeline(&write_pipeline);
    cmd.dispatch(WIDTH / 8, HEIGHT / 8, 1);

    cmd.barrier(
        Stage::COMPUTE,
        Access::SHADER_WRITE,
        Stage::COMPUTE,
        Access::SHADER_SAMPLED_READ | Access::SHADER_WRITE,
    );

    cmd.bind_compute_pipeline(&check_pipeline);
    cmd.set_bindless_root(result.gpu(), result.gpu());
    cmd.dispatch(WIDTH / 8, HEIGHT / 8, 1);

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

    let host = result.host().expect("result must have a host view");
    let pixels: &[[f32; 4]] =
        unsafe { std::slice::from_raw_parts(host.typed::<[f32; 4]>(), (WIDTH * HEIGHT) as usize) };
    for (index, pixel) in pixels.iter().enumerate() {
        let (x, y) = (index % WIDTH as usize, index / WIDTH as usize);
        let expected = [x as f32, y as f32, 0.5, 1.0];
        // The image is RGBA16F; these values are exact in half precision.
        assert_eq!(
            pixel, &expected,
            "texel ({x}, {y}) must round-trip through the storage image"
        );
    }
}

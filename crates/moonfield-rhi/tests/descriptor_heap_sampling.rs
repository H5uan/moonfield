//! Headless end-to-end test for the bindless 2.0 sampling path (route A):
//! a texture created through `Texture::bindless` is sampled by a compute
//! shader that reads it through the native `ResourceDescriptorHeap` /
//! `SamplerDescriptorHeap` syntax (`spvDescriptorHeapEXT` capability), with
//! *no* descriptor set layout on the pipeline — the heap binding alone feeds
//! the shader. Validates the whole chain on the real driver: heap write →
//! `cmd_bind` → untyped heap access → sampled color readback.

mod common;

use moonfield_rhi::bindless::GpuAllocation;
use moonfield_rhi::vulkan::bindless::{ComputePipeline, Memory};
use moonfield_rhi::{
    CommandBufferUsage, CommandPool, Compiler, Device, Format, Instance, SamplerDesc, ShaderModule,
    Texture,
};

/// Sample one pixel per thread from the heap texture (thread i samples column
/// i of the 4x4 texture); the result goes out through a root BDA pointer, so
/// the shader needs no descriptor bindings at all besides the heaps.
const SAMPLER_KERNEL: &str = r#"
[shader("compute")]
[numthreads(4, 1, 1)]
void main(uint3 tid : SV_DispatchThreadID,
          Ptr<float4, Access.ReadWrite> out)
{
    Texture2D tex = ResourceDescriptorHeap[NonUniformResourceIndex(0)];
    SamplerState s = SamplerDescriptorHeap[NonUniformResourceIndex(0)];
    float2 uv = float2((float(tid.x) + 0.5) / 4.0, 0.5);
    out[tid.x] = tex.SampleLevel(s, uv, 0);
}
"#;

#[test]
fn heap_texture_sampling_roundtrip() {
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

    // The sampler slot must hold a valid sampler before the shader samples:
    // the heap encodes the create info straight into slot 0.
    eprintln!("MARK: device ok");
    let heap = device.descriptor_heap();
    let sampler = heap.alloc_sampler_slot().expect("sampler slot");
    heap.write_samplers(&[(sampler, SamplerDesc::default())])
        .expect("write sampler");

    // A 4x4 solid-red texture lands in texture slot 0.
    let mut pixels = Vec::with_capacity(4 * 4 * 4);
    for _ in 0..4 * 4 {
        pixels.extend_from_slice(&[255, 0, 0, 255]);
    }
    eprintln!("MARK: heap writes ok");
    let mut uploader = moonfield_rhi::FrameUploader::new(&device, moonfield_rhi::UPLOAD_ARENA_SIZE)
        .expect("uploader");
    let texture = Texture::bindless(&device, &mut uploader, 4, 4, Format::R8G8B8A8Unorm, &pixels)
        .expect("bindless texture");
    assert_eq!(texture.handle(), Some(moonfield_rhi::TextureHandle(0)));
    // Submit the queued upload before dispatching the sampler, so the pixels
    // are actually in GPU memory (FrameUploader submits on `end_frame`).
    uploader.end_frame().expect("submit uploads");
    eprintln!("MARK: upload submitted");

    // The pipeline has no descriptor set layout at all: the heap binding
    // alone must feed the shader's ResourceDescriptorHeap accesses.
    eprintln!("MARK: texture ok");
    let compiler = Compiler::new().expect("compiler");
    let spirv = compiler
        .compile_source_to_spirv_with_capabilities(
            "heap_sampler",
            SAMPLER_KERNEL,
            "main",
            &["spvDescriptorHeapEXT"],
        )
        .expect("shader compilation");
    eprintln!("MARK: compiled");
    let module = ShaderModule::from_spirv(&device, &spirv).expect("shader module");
    let pipeline = ComputePipeline::new(&device, &module).expect("compute pipeline");

    let out = GpuAllocation::new(&device, 4 * 16, Memory::Default).expect("out allocation");

    eprintln!("MARK: pipeline ok");
    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("command buffer");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    heap.cmd_bind(&cmd).expect("bind heaps");
    eprintln!("MARK: heaps bound");
    cmd.bind_compute_pipeline(pipeline.raw());
    cmd.set_bindless_root(pipeline.layout(), out.gpu(), out.gpu());
    cmd.dispatch(4, 1, 1);
    cmd.end().expect("end");
    eprintln!("MARK: recorded");

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

    // Every thread sampled the same solid-red texture: expect (1,0,0,1).
    let host = out.host().expect("out must be host-visible");
    let samples = unsafe { std::slice::from_raw_parts(host.typed::<[f32; 4]>(), 4) };
    for (i, sample) in samples.iter().enumerate() {
        assert!(
            (sample[0] - 1.0).abs() < 0.02 && sample[1].abs() < 0.02 && sample[2].abs() < 0.02,
            "sample {i} = {sample:?}, expected solid red"
        );
    }
}

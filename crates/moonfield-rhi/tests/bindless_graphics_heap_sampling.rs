//! Headless end-to-end test for bindless heap sampling in the fragment stage:
//! a graphics pipeline with no descriptor set layout samples a heap texture
//! through `ResourceDescriptorHeap` / `SamplerDescriptorHeap` and multiplies
//! it by a tint read through a root BDA pointer (one pushed address).
//! Validates the whole chain on the real driver: heap write → `cmd_bind` →
//! fragment-stage untyped heap access → sampled color readback.

mod common;

use moonfield_rhi::{
    AttachmentLayout, Buffer, BufferUsage, ClearValue, CommandBufferUsage, CommandPool, Compiler,
    Device, Format, GpuAllocation, GraphicsPipeline, Instance, LoadOp, Memory, OffscreenTarget,
    Rect2d, RenderAttachment, RenderPassDesc, SamplerDesc, ShaderModule, StoreOp, Texture,
    VertexAttribute, VertexBufferLayout, VertexFormat,
};

const SIZE: u32 = 64;

/// Fullscreen triangle; the UV spans the 4x4 test texture across the screen.
const VERTEX_SHADER: &str = r#"
struct VsInput
{
    float2 position : POSITION;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

[shader("vertex")]
VsOutput main(VsInput input)
{
    VsOutput output;
    output.position = float4(input.position, 0.0, 1.0);
    output.uv = input.position * 0.5 + 0.5;
    return output;
}
"#;

/// Samples heap texture slot 0 with heap sampler slot 0 and applies a tint
/// read through the root pointer. The pipeline has no descriptor set layout;
/// the bound heaps alone feed the shader.
const FRAGMENT_SHADER: &str = r#"
struct PsInput
{
    float2 uv : TEXCOORD0;
};

[shader("fragment")]
float4 main(PsInput input, Ptr<float4, Access.Read> tint) : SV_TARGET
{
    Texture2D tex = ResourceDescriptorHeap[NonUniformResourceIndex(0)];
    SamplerState s = SamplerDescriptorHeap[NonUniformResourceIndex(0)];
    return tex.SampleLevel(s, input.uv, 0) * tint[0];
}
"#;

#[test]
fn fragment_heap_sampling_roundtrip() {
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
    let heap = device.descriptor_heap();
    let sampler = heap.alloc_sampler_slot().expect("sampler slot");
    heap.write_samplers(&[(sampler, SamplerDesc::default())])
        .expect("write sampler");

    // A 4x4 solid-red texture lands in texture slot 0.
    let mut pixels = Vec::with_capacity(4 * 4 * 4);
    for _ in 0..4 * 4 {
        pixels.extend_from_slice(&[255, 0, 0, 255]);
    }
    let mut uploader = moonfield_rhi::FrameUploader::new(&device, moonfield_rhi::UPLOAD_ARENA_SIZE)
        .expect("uploader");
    let texture = Texture::bindless(&device, &mut uploader, 4, 4, Format::R8G8B8A8Unorm, &pixels)
        .expect("bindless texture");
    assert_eq!(texture.handle(), Some(moonfield_rhi::TextureHandle(0)));
    // Submit the queued upload before drawing, so the pixels are actually in
    // GPU memory (FrameUploader submits on `end_frame`).
    uploader.end_frame().expect("submit uploads");

    // White tint behind a BDA root pointer: the fragment shader multiplies the
    // heap sample by `tint[0]`, so white leaves the texture's red untouched —
    // a black pixel means the root pointer broke, garbage means the heap did.
    let tint = GpuAllocation::new(&device, 16, Memory::Default).expect("tint allocation");
    unsafe {
        *tint.host().expect("tint host view").typed::<[f32; 4]>() = [1.0; 4];
    }

    let compiler = Compiler::new().expect("compiler");
    let vertex_spirv = compiler
        .compile_source_to_spirv("fullscreen_vs", VERTEX_SHADER, "main")
        .expect("vertex shader");
    let fragment_spirv = compiler
        .compile_source_to_spirv_with_capabilities(
            "heap_sampler_fs",
            FRAGMENT_SHADER,
            "main",
            &["spvDescriptorHeapEXT"],
        )
        .expect("fragment shader");
    let vertex_shader = ShaderModule::from_compiled(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_compiled(&device, &fragment_spirv).expect("fs module");

    let target = OffscreenTarget::new(&device, SIZE, SIZE, Format::B8G8R8A8Unorm).expect("target");
    // Descriptor-heap pipeline: the fragment entry point's `Ptr<float4>` root
    // parameter is delivered through push data.
    let pipeline = GraphicsPipeline::new_with_options(
        &device,
        &[Format::B8G8R8A8Unorm],
        None,
        &vertex_shader,
        &fragment_shader,
        &VertexBufferLayout {
            stride: std::mem::size_of::<[f32; 2]>() as u32,
            attributes: vec![VertexAttribute {
                location: 0,
                format: VertexFormat::Float32x2,
                offset: 0,
            }],
        },
    )
    .expect("pipeline");

    let vertices: [[f32; 2]; 3] = [[-1.0, -1.0], [3.0, -1.0], [-1.0, 3.0]];
    let vertex_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(&vertices) as u64,
        BufferUsage::VERTEX,
        moonfield_rhi::Memory::Default,
    )
    .expect("vertex buffer");
    vertex_buffer.upload(&device, &vertices).expect("upload");

    let command_pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut command_buffer = command_pool.allocate_command_buffer().expect("cmd");
    command_buffer
        .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    // Heap binding is command-buffer scoped and bind-point agnostic.
    heap.cmd_bind(&command_buffer).expect("bind heaps");
    let color_attachment = RenderAttachment {
        view: target.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
    };
    command_buffer.begin_rendering(&RenderPassDesc {
        render_area: Rect2d::full(SIZE, SIZE),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment: None,
    });
    command_buffer.bind_graphics_pipeline(&pipeline);
    command_buffer.bind_vertex_buffers(0, &[&vertex_buffer], &[0]);
    // The tint pointer is the fragment root data: the 64-bit address pushed
    // through push data (the push-constant storage class).
    command_buffer.push_data(0, &tint.gpu().as_raw().to_le_bytes());
    command_buffer.draw(3, 1, 0, 0);
    command_buffer.end_rendering();
    command_buffer.end().expect("end");

    let command_buffers = [command_buffer.raw()];
    let submit_info = ash::vk::SubmitInfo::default().command_buffers(&command_buffers);
    unsafe {
        device
            .raw()
            .queue_submit(
                device.graphics_queue(),
                std::slice::from_ref(&submit_info),
                ash::vk::Fence::null(),
            )
            .expect("submit");
        device
            .raw()
            .queue_wait_idle(device.graphics_queue())
            .expect("wait idle");
    }

    // The triangle covers the whole target; every pixel is red texture × white
    // tint. Check the center pixel (BGRA byte order).
    let pixels = target.read_pixels(&device).expect("readback");
    let center = &pixels[((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize..][..4];
    assert!(
        center[2] > 200 && center[0] < 60 && center[1] < 60 && center[3] == 255,
        "center pixel = {center:?}, expected solid red (heap sample × white tint)"
    );
}

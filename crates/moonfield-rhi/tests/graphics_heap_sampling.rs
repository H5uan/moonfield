//! Minimal repro: heap sampling from a *graphics* pipeline.
//!
//! The compute sibling (`descriptor_heap_sampling`) passes on the real
//! driver; the same heap read in a fragment shader must too. A fullscreen
//! triangle with no vertex inputs, no push data, and no sampler — the
//! fragment stage only loads texel (0,0) of heap slot 0. Deliberately
//! independent of the editor/egui machinery.

mod common;

use moonfield_rhi::bindless::{GpuAllocation, Memory};
use moonfield_rhi::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CommandPool, Compiler, Device, Format,
    FrameUploader, GraphicsPipeline, Instance, LoadOp, OffscreenTarget, PipelineOptions, Rect2d,
    RenderAttachment, RenderPassDesc, ShaderModule, StoreOp, Texture, VertexBufferLayout,
    UPLOAD_ARENA_SIZE,
};

const VERTEX: &str = r#"
struct VsOutput { float4 position : SV_POSITION; };
[shader("vertex")]
VsOutput main(uint vid : SV_VertexID)
{
    VsOutput o;
    float2 p = float2(float((vid << 1) & 2u), float(vid & 2u));
    o.position = float4(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}
"#;

const FRAGMENT: &str = r#"
[[vk::binding(0, 0)]]
Texture2D tex;
[shader("fragment")]
float4 main() : SV_TARGET
{
    return tex.Load(int3(int2(0, 0), 0));
}
"#;

#[test]
// AMD 26.8.1 (RX 9070 XT) loses the device on any graphics-stage heap
// descriptor read; kept as the minimal repro for the driver report. Run
// explicitly with `--ignored`.
#[ignore = "driver bug: graphics-stage descriptor heap read faults (AMD 26.8.1)"]
fn graphics_heap_sampling_roundtrip() {
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

    // Slot 0: a 4x4 solid-red texture (created before the render target so it
    // claims heap slot 0).
    let mut pixels = Vec::with_capacity(4 * 4 * 4);
    for _ in 0..4 * 4 {
        pixels.extend_from_slice(&[255, 0, 0, 255]);
    }
    let mut uploader = FrameUploader::new(&device, UPLOAD_ARENA_SIZE).expect("uploader");
    let texture = Texture::bindless(&device, &mut uploader, 4, 4, Format::R8G8B8A8Unorm, &pixels)
        .expect("bindless texture");
    assert_eq!(texture.handle(), Some(moonfield_rhi::TextureHandle(0)));
    uploader.end_frame().expect("submit uploads");

    let compiler = Compiler::new().expect("compiler");
    let vertex_spirv = compiler
        .compile_source_to_spirv("vs", VERTEX, "main")
        .expect("vertex shader");
    let fragment_spirv = compiler
        .compile_source_to_spirv("fs", FRAGMENT, "main")
        .expect("fragment shader");
    let vertex_shader = ShaderModule::from_spirv(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_spirv(&device, &fragment_spirv).expect("fs module");

    let vertex_layout = VertexBufferLayout {
        stride: 0,
        attributes: vec![],
    };
    // The fragment's Texture2D binding is resolved against the resource heap
    // through a push-index mapping: slot index comes from push data @ 0.
    let heap_mappings = [moonfield_rhi::HeapMapping {
        set: 0,
        binding: 0,
        resource: moonfield_rhi::HeapMappingResource::SampledImage,
        push_offset: 0,
        sampler_push_offset: 0,
    }];
    let pipeline = GraphicsPipeline::new_with_options(
        &device,
        &[Format::B8G8R8A8Unorm],
        None,
        &vertex_shader,
        &fragment_shader,
        &vertex_layout,
        &[],
        &PipelineOptions {
            descriptor_heap: true,
            heap_mappings: &heap_mappings,
            ..PipelineOptions::default()
        },
    )
    .expect("pipeline");

    let target = OffscreenTarget::new(&device, 64, 64, Format::B8G8R8A8Unorm).expect("target");
    let heap = device.descriptor_heap();

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("cmd");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    heap.cmd_bind(&cmd).expect("bind heaps");
    let color = RenderAttachment {
        view: target.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
    };
    cmd.begin_rendering(&RenderPassDesc {
        render_area: Rect2d::full(64, 64),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color),
        depth_attachment: None,
    });
    cmd.bind_graphics_pipeline(&pipeline);
    // The mapping's push index: texture slot 0.
    cmd.push_data(0, &0u32.to_le_bytes());
    cmd.draw(3, 1, 0, 0);
    cmd.end_rendering();
    cmd.end().expect("end");
    device.submit_and_wait(&[&cmd]).expect("submit and wait");

    let readback = target.read_pixels(&device).expect("readback");
    let px = &readback[0..4];
    // BGRA: solid red = (0, 0, 255, 255), within unorm rounding.
    assert!(
        px[2] > 200 && px[0] < 60 && px[1] < 60,
        "heap-sampled texel must be red, got BGRA {px:?}"
    );
}

/// The same heap read, but the slot holds a *storage buffer* descriptor and
/// the fragment shader reads it through the untyped heap path. Isolates
/// image-descriptor reads from buffer-descriptor reads in the graphics stage.
#[test]
// Same driver bug as `graphics_heap_sampling_roundtrip`; this variant reads
// a storage-buffer descriptor instead of an image descriptor.
#[ignore = "driver bug: graphics-stage descriptor heap read faults (AMD 26.8.1)"]
fn graphics_heap_buffer_descriptor_read() {
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

    // Slot 0: a 16-byte storage buffer holding one red float4.
    let heap = device.descriptor_heap();
    let slot = heap.alloc_image_slot().expect("slot");
    let buffer = GpuAllocation::new(&device, 16, Memory::Default).expect("buffer");
    unsafe {
        *buffer.host().expect("host-visible").typed::<[f32; 4]>() = [1.0, 0.0, 0.0, 1.0];
    }
    heap.write_buffer_descriptors(&[(
        slot,
        ash::vk::DeviceAddressRangeEXT {
            address: buffer.gpu().as_raw(),
            size: 16,
        },
    )])
    .expect("write buffer descriptor");

    const FRAGMENT: &str = r#"
[shader("fragment")]
float4 main() : SV_TARGET
{
    StructuredBuffer<float4> buf = ResourceDescriptorHeap[NonUniformResourceIndex(0)];
    return buf[0];
}
"#;

    let compiler = Compiler::new().expect("compiler");
    let vertex_spirv = compiler
        .compile_source_to_spirv("vs", VERTEX, "main")
        .expect("vertex shader");
    let fragment_spirv = compiler
        .compile_source_to_spirv_with_capabilities(
            "fs",
            FRAGMENT,
            "main",
            &["spvDescriptorHeapEXT"],
        )
        .expect("fragment shader");
    let vertex_shader = ShaderModule::from_spirv(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_spirv(&device, &fragment_spirv).expect("fs module");

    let vertex_layout = VertexBufferLayout {
        stride: 0,
        attributes: vec![],
    };
    let pipeline = GraphicsPipeline::new_with_options(
        &device,
        &[Format::B8G8R8A8Unorm],
        None,
        &vertex_shader,
        &fragment_shader,
        &vertex_layout,
        &[],
        &PipelineOptions {
            descriptor_heap: true,
            ..PipelineOptions::default()
        },
    )
    .expect("pipeline");

    let target = OffscreenTarget::new(&device, 64, 64, Format::B8G8R8A8Unorm).expect("target");

    let pool = CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut cmd = pool.allocate_command_buffer().expect("cmd");
    cmd.begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    heap.cmd_bind(&cmd).expect("bind heaps");
    let color = RenderAttachment {
        view: target.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
    };
    cmd.begin_rendering(&RenderPassDesc {
        render_area: Rect2d::full(64, 64),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color),
        depth_attachment: None,
    });
    cmd.bind_graphics_pipeline(&pipeline);
    cmd.draw(3, 1, 0, 0);
    cmd.end_rendering();
    cmd.end().expect("end");
    device.submit_and_wait(&[&cmd]).expect("submit and wait");

    let readback = target.read_pixels(&device).expect("readback");
    let px = &readback[0..4];
    assert!(
        px[2] > 200 && px[0] < 60 && px[1] < 60,
        "heap buffer read must be red, got BGRA {px:?}"
    );
}

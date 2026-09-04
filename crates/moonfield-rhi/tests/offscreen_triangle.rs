//! Pixel-verified offscreen draw test for Lunar Mare.
//!
//! `headless_triangle` only checks that commands record; this test actually
//! renders a triangle into an `OffscreenTarget` and reads the pixels back.
//! Skips gracefully on machines without a Vulkan driver.

use ash::vk;
use moonfield_rhi::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CommandPool, Compiler, Device, Format,
    GpuAllocation, GraphicsPipeline, Instance, LoadOp, Memory, OffscreenTarget, Rect2d,
    RenderAttachment, RenderPassDesc, RootBinder, ShaderModule, StoreOp,
};

mod common;
const SIZE: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

#[test]
fn offscreen_triangle_rasterizes() {
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

    // Pull-based vertex path: the only stage input is SV_VertexID; geometry is
    // fetched through the `vertices` root pointer (push data, not a bound buffer).
    let vertex_source = r#"
struct VertexData
{
    float3 position;
    float3 color;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float3 color : COLOR;
};

[shader("vertex")]
VsOutput main(uint vid : SV_VertexID, Ptr<VertexData> vertices)
{
    VsOutput output;
    output.position = float4(vertices[vid].position, 1.0);
    output.color = vertices[vid].color;
    return output;
}
"#;

    let fragment_source = r#"
struct PsInput
{
    float3 color : COLOR;
};

[shader("fragment")]
float4 main(PsInput input) : SV_TARGET
{
    return float4(input.color, 1.0);
}
"#;

    let vertex_spirv = compiler
        .compile_source_to_spirv("triangle_vs", vertex_source, "main")
        .expect("vertex shader");
    let fragment_spirv = compiler
        .compile_source_to_spirv("triangle_fs", fragment_source, "main")
        .expect("fragment shader");

    // The vertex array's device address is delivered through push data; its
    // placement comes from the reflected entry point, not a hand-synced constant.
    let reflection = compiler
        .compile_source_to_reflection("triangle_vs", vertex_source, "main")
        .expect("vertex shader reflection");
    let binder = RootBinder::new(&reflection, "main").expect("root binder");
    let vertices_place = binder.pointer_param("vertices").expect("vertices place");
    drop(reflection);

    let vertex_shader = ShaderModule::from_compiled(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_compiled(&device, &fragment_spirv).expect("fs module");

    let target = OffscreenTarget::new(&device, SIZE, SIZE, Format::B8G8R8A8Unorm).expect("target");
    let pipeline = GraphicsPipeline::new(
        &device,
        Format::B8G8R8A8Unorm,
        &vertex_shader,
        &fragment_shader,
    )
    .expect("pipeline");

    // A big triangle covering most of the target.
    let vertices = [
        Vertex {
            position: [-0.8, -0.8, 0.0],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.8, -0.8, 0.0],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.0, 0.8, 0.0],
            color: [1.0, 0.0, 0.0],
        },
    ];
    let vertex_alloc = GpuAllocation::new(
        &device,
        std::mem::size_of_val(&vertices) as u64,
        Memory::Default,
    )
    .expect("vertex allocation");
    // SAFETY: host-visible allocation, written once before recording.
    unsafe {
        std::ptr::copy_nonoverlapping(
            vertices.as_ptr(),
            vertex_alloc.host().expect("host view").typed::<Vertex>(),
            vertices.len(),
        );
    }

    let command_pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("pool");
    let mut command_buffer = command_pool.allocate_command_buffer().expect("cmd");
    command_buffer
        .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("begin");
    let color_attachment = RenderAttachment {
        view: target.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
    };
    let begin_info = RenderPassDesc {
        render_area: Rect2d::full(SIZE, SIZE),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment: None,
    };
    command_buffer.begin_rendering(&begin_info);
    command_buffer.bind_graphics_pipeline(&pipeline);
    // The vertex allocation's address is pushed; the shader pulls vertices
    // through it. One push before the draw: the array does not change.
    let bytes = vertices_place
        .pointer_bytes(vertex_alloc.gpu().as_raw())
        .expect("vertices pointer encode");
    command_buffer.push_data(vertices_place.offset as u32, &bytes);
    command_buffer.draw(3, 1, 0, 0);
    command_buffer.end_rendering();
    command_buffer.end().expect("end");

    let command_buffers = [command_buffer.raw()];
    let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
    // SAFETY: the command buffer is fully recorded and the queue is valid.
    unsafe {
        device
            .raw()
            .queue_submit(
                device.graphics_queue(),
                std::slice::from_ref(&submit_info),
                vk::Fence::null(),
            )
            .expect("submit");
        device
            .raw()
            .queue_wait_idle(device.graphics_queue())
            .expect("wait idle");
    }

    let pixels = target.read_pixels(&device).expect("readback");
    let red = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[2] > 200 && px[0] < 60 && px[1] < 60 && px[3] == 255)
        .count();
    assert!(red > 100, "triangle did not rasterize: {red} red pixels");
}

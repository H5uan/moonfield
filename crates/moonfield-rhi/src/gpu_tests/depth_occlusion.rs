//! Depth-occlusion test for Lunar Mare's reverse-Z depth path.
//!
//! Renders two fully overlapping quads into an `OffscreenTarget::new_with_depth`
//! target with reverse-Z depth testing enabled per draw through
//! `CommandBuffer::set_depth_state`: the near (red) quad is drawn *first*, the
//! far (blue) quad second. Without depth testing the blue
//! quad would overwrite the red one; with reverse-Z depth (clear 0.0, compare
//! `GREATER_OR_EQUAL`) the near quad must win. Skips gracefully on machines
//! without a Vulkan driver.

use crate::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CommandPool, CompareOp, Compiler, CullMode,
    CullState, DepthState, Device, Format, FrontFace, GpuAllocation, GraphicsPipeline, Instance,
    LoadOp, Memory, OffscreenTarget, Rect2d, RenderAttachment, RenderPassDesc, RootBinder,
    ShaderModule, StoreOp,
};
use ash::vk;

use super::common;
const SIZE: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];

/// A full-screen quad (two triangles) at NDC depth `z` with a flat color.
fn quad(z: f32, color: [f32; 3]) -> [Vertex; 6] {
    let corners = [
        [-1.0, -1.0],
        [1.0, -1.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [1.0, 1.0],
        [-1.0, 1.0],
    ];
    corners.map(|[x, y]| Vertex {
        position: [x, y, z],
        color,
    })
}

#[test]
fn depth_test_near_quad_occludes_far_quad() {
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
        .compile_source_to_spirv("quad_vs", vertex_source, "main")
        .expect("vertex shader");
    let fragment_spirv = compiler
        .compile_source_to_spirv("quad_fs", fragment_source, "main")
        .expect("fragment shader");

    // The vertex array's device address is delivered through push data; its
    // placement comes from the reflected entry point, not a hand-synced constant.
    let reflection = compiler
        .compile_source_to_reflection("quad_vs", vertex_source, "main")
        .expect("vertex shader reflection");
    let binder = RootBinder::new(&reflection, "main").expect("root binder");
    let vertices_place = binder.pointer_param("vertices").expect("vertices place");
    drop(reflection);

    let vertex_shader = ShaderModule::from_compiled(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_compiled(&device, &fragment_spirv).expect("fs module");

    let target = OffscreenTarget::new_with_depth(&device, SIZE, SIZE, Format::B8G8R8A8Unorm)
        .expect("target");
    assert!(target.has_depth(), "depth target must report has_depth");

    let pipeline = GraphicsPipeline::new_with_options(
        &device,
        &[Format::B8G8R8A8Unorm],
        Some(Format::D32Sfloat),
        &vertex_shader,
        &fragment_shader,
    )
    .expect("pipeline");

    // Reverse-Z: larger NDC z is closer. Draw the near red quad first and the
    // far blue quad second; depth testing must keep red visible.
    let near = quad(0.8, RED);
    let far = quad(0.5, BLUE);
    let vertices = [&near[..], &far[..]].concat();
    let vertex_alloc = GpuAllocation::new(
        &device,
        std::mem::size_of_val(vertices.as_slice()) as u64,
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
    // Reverse-Z: the far plane is 0, so depth clears to 0.0.
    let depth_attachment = target.depth_view().map(|view| RenderAttachment {
        view,
        layout: AttachmentLayout::DepthStencil,
        load: LoadOp::Clear,
        store: StoreOp::Discard,
        clear: ClearValue::DepthStencil {
            depth: 0.0,
            stencil: 0,
        },
    });
    let begin_info = RenderPassDesc {
        render_area: Rect2d::full(SIZE, SIZE),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment,
    };
    command_buffer.begin_rendering(&begin_info);
    // Reverse-Z depth testing: near (larger NDC z) wins.
    command_buffer.set_depth_state(DepthState {
        test_enable: true,
        write_enable: true,
        compare_op: CompareOp::GreaterOrEqual,
    });
    command_buffer.set_cull_state(CullState {
        cull_mode: CullMode::None,
        front_face: FrontFace::CounterClockwise,
    });
    command_buffer.bind_graphics_pipeline(&pipeline);
    // The vertex allocation's address is pushed once; the shader pulls both
    // quads through it — each draw's `first_vertex` offsets SV_VertexID into
    // the array (near quad at 0, far quad at 6).
    let bytes = vertices_place
        .pointer_bytes(vertex_alloc.gpu().as_raw())
        .expect("vertices pointer encode");
    command_buffer.push_data(vertices_place.offset as u32, &bytes);
    command_buffer.draw(6, 1, 0, 0); // near red quad first
    command_buffer.draw(6, 1, 6, 0); // far blue quad second — must lose
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
    let is_red = |px: &[u8; 4]| px[2] > 200 && px[0] < 60 && px[1] < 60 && px[3] == 255;
    let is_blue = |px: &[u8; 4]| px[0] > 200 && px[1] < 60 && px[2] < 60 && px[3] == 255;
    let chunks = pixels.as_chunks::<4>().0;
    let red = chunks.iter().filter(|px| is_red(px)).count();
    let blue = chunks.iter().filter(|px| is_blue(px)).count();

    // The quads cover the whole target; every pixel must be the near red one.
    // Any blue means the far quad won — depth testing is broken.
    assert_eq!(
        blue, 0,
        "far quad leaked through depth test: {blue} blue pixels"
    );
    assert!(
        red > (SIZE * SIZE * 9 / 10) as usize,
        "near quad did not cover the target: {red} red pixels"
    );

    // The depth path must survive a resize (recreates color + depth images).
    target_resize_smoke(&device);
}

/// Resize a depth target and confirm the recreated framebuffer still records.
fn target_resize_smoke(device: &Device) {
    let mut target =
        OffscreenTarget::new_with_depth(device, SIZE, SIZE, Format::B8G8R8A8Unorm).expect("target");
    target.resize(device, 32, 32).expect("resize with depth");
    assert!(target.has_depth());
    assert_eq!(target.extent(), (32, 32));
}

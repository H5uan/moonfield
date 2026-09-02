//! Depth-occlusion test for Lunar Mare's reverse-Z depth path.
//!
//! Renders two fully overlapping quads into an `OffscreenTarget::new_with_depth`
//! target with `PipelineOptions::depth_test` enabled: the near (red) quad is
//! drawn *first*, the far (blue) quad second. Without depth testing the blue
//! quad would overwrite the red one; with reverse-Z depth (clear 0.0, compare
//! `GREATER_OR_EQUAL`) the near quad must win. Skips gracefully on machines
//! without a Vulkan driver.

use ash::vk;
use moonfield_rhi::{
    AttachmentLayout, Buffer, BufferUsage, ClearValue, CommandBufferUsage, CommandPool, CompareOp,
    Compiler, CullMode, CullState, DepthState, Device, Format, FrontFace, GraphicsPipeline,
    Instance, LoadOp, OffscreenTarget, PipelineOptions, Rect2d, RenderAttachment, RenderPassDesc,
    ShaderModule, StoreOp, VertexAttribute, VertexBufferLayout, VertexFormat,
};

mod common;
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
    let vertex_spirv = compiler
        .compile_source_to_spirv(
            "quad_vs",
            r#"
struct VsInput
{
    float3 position : POSITION;
    float3 color : COLOR;
};

struct VsOutput
{
    float4 position : SV_POSITION;
    float3 color : COLOR;
};

[shader("vertex")]
VsOutput main(VsInput input)
{
    VsOutput output;
    output.position = float4(input.position, 1.0);
    output.color = input.color;
    return output;
}
"#,
            "main",
        )
        .expect("vertex shader");
    let fragment_spirv = compiler
        .compile_source_to_spirv(
            "quad_fs",
            r#"
struct PsInput
{
    float3 color : COLOR;
};

[shader("fragment")]
float4 main(PsInput input) : SV_TARGET
{
    return float4(input.color, 1.0);
}
"#,
            "main",
        )
        .expect("fragment shader");
    let vertex_shader = ShaderModule::from_spirv(&device, &vertex_spirv).expect("vs module");
    let fragment_shader = ShaderModule::from_spirv(&device, &fragment_spirv).expect("fs module");

    let target = OffscreenTarget::new_with_depth(&device, SIZE, SIZE, Format::B8G8R8A8Unorm)
        .expect("target");
    assert!(target.has_depth(), "depth target must report has_depth");

    let pipeline = GraphicsPipeline::new_with_options(
        &device,
        &[Format::B8G8R8A8Unorm],
        Some(Format::D32Sfloat),
        &vertex_shader,
        &fragment_shader,
        &VertexBufferLayout {
            stride: std::mem::size_of::<Vertex>() as u32,
            attributes: vec![
                VertexAttribute {
                    location: 0,
                    format: VertexFormat::Float32x3,
                    offset: 0,
                },
                VertexAttribute {
                    location: 1,
                    format: VertexFormat::Float32x3,
                    offset: 12,
                },
            ],
        },
        &PipelineOptions::default(),
    )
    .expect("pipeline");

    // Reverse-Z: larger NDC z is closer. Draw the near red quad first and the
    // far blue quad second; depth testing must keep red visible.
    let near = quad(0.8, RED);
    let far = quad(0.5, BLUE);
    let vertices = [&near[..], &far[..]].concat();
    let vertex_buffer = Buffer::new(
        &device,
        std::mem::size_of_val(vertices.as_slice()) as u64,
        BufferUsage::VERTEX,
        gpu_allocator::MemoryLocation::CpuToGpu,
    )
    .expect("vertex buffer");
    vertex_buffer
        .upload(&device, vertices.as_slice())
        .expect("upload");

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
    command_buffer.bind_vertex_buffers(0, &[&vertex_buffer], &[0]);
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

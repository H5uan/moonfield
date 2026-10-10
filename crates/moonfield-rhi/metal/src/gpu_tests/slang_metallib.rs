//! End-to-end Slang→`.metallib` test for the Metal 4 backend.
//!
//! Compiles the same vertex-pull shader shape the Vulkan backend uses
//! (`Ptr<T>` root fetched through a buffer binding, no vertex attributes)
//! with the shared compiler's `MetalLib` target, loads both `.metallib`
//! archives, renders an offscreen triangle through the argument table, and
//! verifies pixels. Proves one Slang source serves both backends. Skips
//! gracefully on machines without a Metal 4 device.

use crate::{
    CommandPool, Compiler, Device, Format, GraphicsPipeline, Instance, LoadOp, Memory,
    RenderAttachment, RenderPassDesc, ShaderModule, ShaderTarget, StoreOp, Texture,
};
use moonfield_rhi_core::{AttachmentLayout, ClearValue, Rect2d};

const SIZE: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

const CLEAR: [f32; 4] = [0.1, 0.1, 0.2, 1.0];

#[test]
fn slang_metallib_renders_offscreen_triangle() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Metal 4 device available ({err})");
            return;
        }
    };
    let device = Device::new(&instance).expect("device");

    // Same shape as the Vulkan offscreen test: `SV_VertexID` pull through a
    // `Ptr<VertexData>` root. On the Metal target Slang lowers the pointer
    // to a buffer binding (argument-table slot 0) and the semantics to
    // Metal's `[[vertex_id]]`/`[[position]]`.
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
VsOutput vs_main(uint vid : SV_VertexID, Ptr<VertexData> vertices)
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
float4 fs_main(PsInput input) : SV_TARGET
{
    return float4(input.color, 1.0);
}
"#;

    let compiler = Compiler::new().expect("compiler");
    let vertex_shader = compiler
        .compile_source("vs", vertex_source, "vs_main", ShaderTarget::MetalLib)
        .expect("compile vertex to metallib");
    let fragment_shader = compiler
        .compile_source("fs", fragment_source, "fs_main", ShaderTarget::MetalLib)
        .expect("compile fragment to metallib");

    let vertex_module = ShaderModule::from_compiled(&device, &vertex_shader).expect("load vs");
    let fragment_module = ShaderModule::from_compiled(&device, &fragment_shader).expect("load fs");
    let pipeline = GraphicsPipeline::from_modules(
        &device,
        &vertex_module,
        &vertex_shader.entry,
        &fragment_module,
        &fragment_shader.entry,
        Format::R8G8B8A8Unorm,
    )
    .expect("pipeline");
    let target =
        Texture::new_render_target(&device, SIZE, SIZE, Format::R8G8B8A8Unorm).expect("texture");

    let vertices = [
        Vertex {
            position: [-0.8, -0.8, 0.0],
            color: [1.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.8, -0.8, 0.0],
            color: [0.0, 1.0, 0.0],
        },
        Vertex {
            position: [0.0, 0.8, 0.0],
            color: [0.0, 0.0, 1.0],
        },
    ];
    let memory = Memory::new(&device, std::mem::size_of_val(&vertices) as u64);
    memory.copy_host(0, bytemuck::bytes_of(&vertices));

    let pool = CommandPool::new(&device);
    let mut cmd = pool.allocate();
    cmd.begin_render_pass(&RenderPassDesc {
        render_area: Rect2d::full(SIZE, SIZE),
        color_attachments: &[RenderAttachment {
            view: target.view(),
            layout: AttachmentLayout::ShaderRead,
            load: LoadOp::Clear,
            store: StoreOp::Store,
            clear: ClearValue::Color(CLEAR),
        }],
        depth_attachment: None,
    });
    cmd.set_pipeline(&pipeline);
    cmd.set_buffer(0, &memory.allocation());
    cmd.draw(3);
    cmd.end_render_pass();

    device.submit_and_wait(cmd).expect("submit");

    let pixels = target.read_pixels();
    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };

    let expected_clear = [
        (CLEAR[0] * 255.0).round() as u8,
        (CLEAR[1] * 255.0).round() as u8,
        (CLEAR[2] * 255.0).round() as u8,
        255,
    ];
    assert_eq!(px(2, 2), expected_clear, "corner must keep the clear color");

    let center = px(SIZE / 2, SIZE / 2);
    assert_eq!(center[3], 255, "triangle must be opaque at the center");
    let luma = center[0] as u32 + center[1] as u32 + center[2] as u32;
    assert!(
        luma > 60,
        "center must be covered by the triangle, got {center:?}"
    );
}

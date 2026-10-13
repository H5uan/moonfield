//! Raster recording contract tests against the facade's public API.
//!
//! `test_raster_triangle_renders` pins the graphics recording surface one
//! spelling across both backends: `GraphicsPipeline::new` with module-carried
//! entries, `RenderPassDesc` with no silently ignored field, `push_data`
//! roots, and `Texture::new_render_target`/`read_pixels` — clear, draw,
//! read back. The assertions separate "the triangle rendered" from "the
//! clear color shows": a black clear with a red triangle.

mod common;

use moonfield_rhi::{
    AttachmentLayout, ClearValue, CommandBufferUsage, Format, GpuAllocation, GraphicsPipeline,
    LoadOp, MemoryResidency, Rect2d, RenderAttachment, RenderPassDesc, RootBinder, ShaderModule,
    StoreOp, Texture,
};

const SIZE: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 3],
}

/// Pull-based vertex path, the engine's only vertex shape: the stage input is
/// `SV_VertexID`; geometry is fetched through the `vertices` root pointer
/// (`push_data`, not a bound vertex buffer).
const VERTEX_SOURCE: &str = r#"
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

const FRAGMENT_SOURCE: &str = r#"
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

/// Clear a target to black, draw a red triangle through the shared raster
/// spelling, and read the pixels back: the corner must stay black and the
/// center must be red — a vacuous pass (nothing rendered) is impossible.
#[test]
fn test_raster_triangle_renders() {
    let Some(device) = common::headless_device() else {
        return;
    };

    let cache = device.shader_cache();
    let vertex_shader = cache
        .compile_source(
            "raster_contract_vs",
            VERTEX_SOURCE,
            "vs_main",
            moonfield_rhi::shader_capabilities(),
            &[],
        )
        .expect("vertex compilation");
    let fragment_shader = cache
        .compile_source(
            "raster_contract_fs",
            FRAGMENT_SOURCE,
            "fs_main",
            moonfield_rhi::shader_capabilities(),
            &[],
        )
        .expect("fragment compilation");

    // The vertex root's placement comes from the reflected entry point, not
    // a hand-synced constant (the same discipline as the compute contract).
    let reflection = cache
        .compile_source_reflection("raster_contract_vs", VERTEX_SOURCE, &["vs_main"])
        .expect("vertex reflection");
    let binder = RootBinder::new(&reflection, "vs_main").expect("root binder");
    let vertices_place = binder.pointer_param("vertices").expect("vertices place");
    drop(reflection);

    let vertex_module = ShaderModule::from_compiled(&device, &vertex_shader).expect("vs module");
    let fragment_module =
        ShaderModule::from_compiled(&device, &fragment_shader).expect("fs module");
    // The unified constructor: one color format, entries carried by the
    // modules.
    let pipeline = GraphicsPipeline::new(
        &device,
        Format::R8G8B8A8Unorm,
        &vertex_module,
        &fragment_module,
    )
    .expect("pipeline");

    let target =
        Texture::new_render_target(&device, SIZE, SIZE, Format::R8G8B8A8Unorm).expect("target");

    // A red triangle covering about a third of the target.
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
        MemoryResidency::CpuWriteOnly,
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

    let cmd = device
        .command_buffer(CommandBufferUsage::ONE_TIME_SUBMIT)
        .expect("command buffer");
    let color_attachment = RenderAttachment {
        view: target.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
    };
    cmd.begin_rendering(&RenderPassDesc {
        render_area: Rect2d::full(SIZE, SIZE),
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment: None,
    });
    cmd.bind_graphics_pipeline(&pipeline);
    let bytes = vertices_place
        .pointer_bytes(vertex_alloc.gpu_ptr().as_raw())
        .expect("vertices pointer encode");
    cmd.push_data(vertices_place.offset as u32, &bytes);
    cmd.draw(3, 1, 0, 0);
    cmd.end_rendering();

    device.submit_and_wait(&[&cmd]).expect("submit");

    let pixels = target.read_pixels(&device).expect("readback");
    let is_red = |px: &[u8; 4]| px[0] > 200 && px[1] < 60 && px[2] < 60 && px[3] == 255;
    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };

    // The corner is outside the triangle: it must keep the black clear.
    assert_eq!(px(2, 2), [0, 0, 0, 255], "corner must keep the clear color");
    // The center is inside the triangle: it must be red, not the clear.
    assert!(
        is_red(&px(SIZE / 2, SIZE / 2)),
        "center must be covered by the triangle, got {:?}",
        px(SIZE / 2, SIZE / 2)
    );
    // The triangle spans roughly a third of the NDC square; require a
    // coverage that only a rendered triangle reaches.
    let covered = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| is_red(px))
        .count();
    assert!(
        covered > (SIZE * SIZE / 4) as usize,
        "triangle coverage too small: {covered} red pixels"
    );
}

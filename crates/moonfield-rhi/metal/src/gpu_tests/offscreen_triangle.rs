//! Pixel-verified offscreen draw test for the Metal 4 backend.
//!
//! Renders a vertex-pulled triangle into an offscreen texture through the
//! per-command-buffer `MTL4ArgumentTable` and reads the pixels back. Skips
//! gracefully on machines without a Metal 4 device.

use crate::{
    CommandPool, Device, Format, GraphicsPipeline, Instance, LoadOp, Memory, RenderAttachment,
    RenderPassDesc, ShaderModule, StoreOp, Texture,
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
fn offscreen_triangle_rasterizes() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Metal 4 device available ({err})");
            return;
        }
    };
    let device = Device::new(&instance).expect("device");

    // Vertex-pulled geometry: the only stage input is vertex_id; positions
    // and colors come from the buffer bound at argument-table slot 0.
    let msl = r#"
#include <metal_stdlib>
using namespace metal;

struct VertexData {
    float3 position;
    float3 color;
};

struct VsOut {
    float4 position [[position]];
    float3 color;
};

vertex VsOut vs_main(uint vid [[vertex_id]],
                     constant VertexData* vertices [[buffer(0)]]) {
    VsOut out;
    out.position = float4(vertices[vid].position, 0.5);
    out.color = vertices[vid].color;
    return out;
}

fragment float4 fs_main(VsOut in [[stage_in]]) {
    return float4(in.color, 1.0);
}
"#;

    let module = ShaderModule::from_msl(&device, msl).expect("compile msl");
    let pipeline = GraphicsPipeline::new(
        &device,
        &module,
        "vs_main",
        "fs_main",
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

//! Swapchain test for the Metal 4 backend: acquire a drawable from a
//! headless `CAMetalLayer`, render through the swapchain's image view,
//! read pixels back, and present. Skips gracefully on machines without a
//! Metal 4 device.

use crate::{
    CommandPool, Device, Format, GraphicsPipeline, Instance, LoadOp, Memory, RenderAttachment,
    RenderPassDesc, Semaphore, ShaderModule, StoreOp, Surface, Swapchain,
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
fn swapchain_acquires_renders_and_presents() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Metal 4 device available ({err})");
            return;
        }
    };
    let device = Device::new(&instance).expect("device");

    let surface = Surface::new_layer(&device, [SIZE as f64, SIZE as f64]);
    let mut swapchain =
        Swapchain::new(&instance, &device, &surface, [SIZE, SIZE]).expect("swapchain");

    let (format, srgb) = swapchain.format_srgb().expect("format");
    assert_eq!(format, Format::B8G8R8A8Unorm);
    assert!(!srgb);
    assert_eq!(swapchain.extent().width, SIZE);
    assert_eq!(swapchain.extent().height, SIZE);

    let module = ShaderModule::from_msl(&device, TRIANGLE_MSL).expect("compile msl");
    let pipeline =
        GraphicsPipeline::new(&device, &module, "vs_main", "fs_main", format).expect("pipeline");

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

    let image_available = Semaphore::new(&device).expect("semaphore");
    let (index, suboptimal) = swapchain
        .acquire_next_image(u64::MAX, &image_available)
        .expect("acquire");
    assert_eq!(index, 0);
    assert!(!suboptimal);

    let view = swapchain.image_view(index);
    let pool = CommandPool::new(&device);
    let mut cmd = pool.allocate();
    cmd.begin_rendering(&RenderPassDesc {
        render_area: Rect2d::full(SIZE, SIZE),
        color_attachments: &[RenderAttachment {
            view: view.clone(),
            layout: AttachmentLayout::Present,
            load: LoadOp::Clear,
            store: StoreOp::Store,
            clear: ClearValue::Color(CLEAR),
        }],
        depth_attachment: None,
    });
    cmd.bind_graphics_pipeline(&pipeline);
    cmd.set_buffer(0, &memory.allocation());
    cmd.draw(3, 1, 0, 0);
    cmd.end_rendering();
    device.submit_and_wait(cmd).expect("submit");

    // Read the drawable's pixels back (the headless layer is created with
    // framebufferOnly off for this).
    let bytes = view.format().bytes_per_pixel();
    let mut pixels = vec![0u8; (SIZE * SIZE) as usize * bytes];
    let region = objc2_metal::MTLRegion {
        origin: objc2_metal::MTLOrigin { x: 0, y: 0, z: 0 },
        size: objc2_metal::MTLSize {
            width: SIZE as usize,
            height: SIZE as usize,
            depth: 1,
        },
    };
    // SAFETY: `pixels` holds width*height*bytes_per_row bytes matching the
    // region; level 0, single-slice 2D drawable texture.
    unsafe {
        use objc2_metal::MTLTexture;
        use std::ptr::NonNull;
        let ptr = NonNull::new(pixels.as_mut_ptr().cast()).unwrap();
        view.texture().getBytes_bytesPerRow_fromRegion_mipmapLevel(
            ptr,
            SIZE as usize * bytes,
            region,
            0,
        );
    }
    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    let expected_clear = [
        (CLEAR[2] * 255.0).round() as u8,
        (CLEAR[1] * 255.0).round() as u8,
        (CLEAR[0] * 255.0).round() as u8,
        255,
    ];
    assert_eq!(px(2, 2), expected_clear, "corner must keep the clear color");
    let center = px(SIZE / 2, SIZE / 2);
    assert_eq!(center[3], 255, "triangle must be opaque at the center");

    let suboptimal = swapchain
        .queue_present(&device, &[], index)
        .expect("present");
    assert!(!suboptimal);
}

const TRIANGLE_MSL: &str = r#"
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

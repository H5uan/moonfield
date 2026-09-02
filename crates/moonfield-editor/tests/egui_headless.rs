//! Headless smoke test for the egui Vulkan backend (`egui_vk`).
//!
//! Renders egui primitives — a text label (managed font texture) and a
//! `ui.image` of a user-registered texture — into an offscreen target, reads
//! the pixels back, and asserts the output is not blank and the user texture
//! sampled through. The second frame exercises the editor's resize flow: the
//! source image is resized, which rewrites its descriptor-heap slot in place
//! while the registered texture id stays valid. Skips gracefully on machines
//! without a Vulkan driver.

use moonfield_editor::egui_vk::{
    EguiFrameResources, EguiOptions, EguiPipeline, EguiTextures, record_egui,
};
use moonfield_rhi::{
    AttachmentLayout, ClearValue, CommandBufferUsage, CommandPool, Format, LoadOp, OffscreenTarget,
    Rect2d, RenderAttachment, RenderDevice, RenderPassDesc, StoreOp,
};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 256;
const CLEAR: [u8; 4] = [0, 0, 0, 255];

#[test]
fn egui_headless_frame_is_not_blank() {
    let render_device = match RenderDevice::new() {
        Ok(render_device) => render_device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };
    let device = render_device.device().clone();

    let target =
        OffscreenTarget::new(&device, WIDTH, HEIGHT, Format::B8G8R8A8Unorm).expect("target");
    // The user-texture source image (the editor viewport's offscreen target
    // analogue), cleared to opaque red each frame.
    let mut user_image =
        OffscreenTarget::new(&device, 8, 8, Format::B8G8R8A8Unorm).expect("user image");

    let mut pipeline = EguiPipeline::new(
        &device,
        Format::B8G8R8A8Unorm,
        false,
        EguiOptions::default(),
    )
    .expect("egui pipeline");
    let mut textures = EguiTextures::new(&render_device).expect("egui textures");
    let mut frames = EguiFrameResources::new(&device, 1).expect("egui frame resources");

    let user_texture =
        textures.register_native_texture(user_image.texture_handle(), user_image.sampler_handle());

    let ctx = egui::Context::default();
    let command_pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("command pool");

    // Frame 1: fresh registration. Frame 2: the source image was resized,
    // rewriting its heap slot in place — the registered id and handles stay
    // valid (the editor's viewport-resize flow).
    for frame in 0..2 {
        if frame == 1 {
            user_image
                .resize(&device, 16, 16)
                .expect("resize user image");
        }

        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH as f32, HEIGHT as f32),
            )),
            ..Default::default()
        };
        let full_output = ctx.run_ui(raw_input, |ui| {
            ui.heading("egui_vk smoke test");
            ui.image((user_texture, egui::vec2(64.0, 64.0)));
        });
        let egui::FullOutput {
            mut textures_delta,
            shapes,
            ..
        } = full_output;

        // Draining marks the delta as applied; epaint 0.36 debug-asserts
        // that a dropped `TexturesDelta` is empty. The test never defers
        // frees (each frame is submitted synchronously), so `free` is
        // discarded here.
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                textures
                    .update_texture(&device, &mut pipeline, id, &delta, 0)
                    .expect("texture upload");
            }
        }
        textures_delta.free.clear();
        let pixels_per_point = 1.0;
        let primitives = ctx.tessellate(shapes, pixels_per_point);
        assert!(!primitives.is_empty(), "egui produced no primitives");
        frames
            .update(&device, 0, &primitives)
            .expect("buffer upload");

        let mut command_buffer = command_pool
            .allocate_command_buffer()
            .expect("command buffer");
        command_buffer
            .begin(CommandBufferUsage::ONE_TIME_SUBMIT)
            .expect("begin");

        // Clear the user-texture source to opaque red.
        clear_to_red(&command_buffer, &user_image);

        // The egui pass into the readback target.
        let color_attachment = RenderAttachment {
            view: target.view(),
            layout: AttachmentLayout::ShaderRead,
            load: LoadOp::Clear,
            store: StoreOp::Store,
            clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
        };
        command_buffer.begin_rendering(&RenderPassDesc {
            render_area: Rect2d::full(WIDTH, HEIGHT),
            layer_count: 1,
            color_attachments: std::slice::from_ref(&color_attachment),
            depth_attachment: None,
        });
        record_egui(
            &command_buffer,
            &pipeline,
            &textures,
            &frames,
            0,
            (WIDTH, HEIGHT),
            pixels_per_point,
            &primitives,
        );
        command_buffer.end_rendering();
        command_buffer.end().expect("end");

        device
            .submit_and_wait(&[&command_buffer])
            .expect("submit and wait");
    }

    // Read back the second frame and verify.
    let pixels = target.read_pixels(&device).expect("readback");
    let non_clear = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| **px != CLEAR)
        .count();
    assert!(
        non_clear > 0,
        "egui frame read back as uniform clear color — nothing was drawn"
    );
    // The user-texture image is opaque red; the `ui.image` quad must show up
    // as red pixels (BGRA byte order) after the resize + rebind.
    let red = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[2] > 200 && px[0] < 60 && px[1] < 60 && px[3] == 255)
        .count();
    assert!(
        red > 1000,
        "user texture did not sample through: only {red} red pixels"
    );
}

/// Record a clear-only rendering pass filling the image with opaque red.
fn clear_to_red(command_buffer: &moonfield_rhi::CommandBuffer, image: &OffscreenTarget) {
    let (width, height) = image.extent();
    let color_attachment = RenderAttachment {
        view: image.view(),
        layout: AttachmentLayout::ShaderRead,
        load: LoadOp::Clear,
        store: StoreOp::Store,
        clear: ClearValue::Color([1.0, 0.0, 0.0, 1.0]),
    };
    command_buffer.begin_rendering(&RenderPassDesc {
        render_area: Rect2d::full(width, height),
        layer_count: 1,
        color_attachments: std::slice::from_ref(&color_attachment),
        depth_attachment: None,
    });
    command_buffer.end_rendering();
}

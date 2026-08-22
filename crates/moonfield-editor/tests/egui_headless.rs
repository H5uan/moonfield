//! Headless smoke test for the egui Vulkan backend (`egui_vk`).
//!
//! Renders egui primitives — a text label (managed font texture), a
//! `ui.image` of a user-registered texture, and clip-rect panels — into an
//! offscreen target, reads the pixels back, and asserts the output is not
//! blank and the user texture sampled through. The second frame exercises
//! the editor's resize flow: the source image is recreated and the user
//! texture id is rebound to the new view. Skips gracefully on machines
//! without a Vulkan driver; Linux CI runs it against lavapipe.

use ash::vk;
use moonfield_editor::egui_vk::{EguiRenderer, RendererOptions};
use moonfield_render::{
    Buffer, BufferUsage, CommandPool, Device, Format, Instance, OffscreenTarget,
};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 256;
const CLEAR: [u8; 4] = [0, 0, 0, 255];

#[test]
fn egui_headless_frame_is_not_blank() {
    let instance = match Instance::new_headless() {
        Ok(instance) => instance,
        Err(err) => {
            eprintln!("skipping: no Vulkan instance available ({err})");
            return;
        }
    };
    let device = match Device::new(&instance, None) {
        Ok(device) => device,
        Err(err) => {
            eprintln!("skipping: no Vulkan device available ({err})");
            return;
        }
    };

    let target =
        OffscreenTarget::new(&device, WIDTH, HEIGHT, Format::B8G8R8A8Unorm).expect("target");
    // The user-texture source image (the editor viewport's offscreen target
    // analogue), cleared to opaque red each frame.
    let mut user_image =
        OffscreenTarget::new(&device, 8, 8, Format::B8G8R8A8Unorm).expect("user image");

    let mut renderer = EguiRenderer::new(
        &device,
        target.render_pass(),
        false,
        1,
        RendererOptions::default(),
    )
    .expect("egui renderer");

    let user_texture = renderer
        .register_native_texture(
            &device,
            &user_image.texture_view(),
            &user_image.sampler_view(),
        )
        .expect("register user texture");

    let ctx = egui::Context::default();
    let command_pool =
        CommandPool::new(&device, device.queue_family_indices().graphics).expect("command pool");

    // Frame 1: fresh registration. Frame 2: the source image was resized
    // (recreating its view) and the texture id rebound — the editor's
    // viewport-resize flow.
    for frame in 0..2 {
        if frame == 1 {
            user_image
                .resize(&device, 16, 16)
                .expect("resize user image");
            renderer
                .update_native_texture(
                    &device,
                    user_texture,
                    &user_image.texture_view(),
                    &user_image.sampler_view(),
                )
                .expect("rebind user texture");
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
                renderer
                    .update_texture(&device, id, &delta)
                    .expect("texture upload");
            }
        }
        textures_delta.free.clear();
        let pixels_per_point = 1.0;
        let primitives = ctx.tessellate(shapes, pixels_per_point);
        assert!(!primitives.is_empty(), "egui produced no primitives");
        renderer
            .update_buffers(&device, 0, &primitives, [WIDTH as f32, HEIGHT as f32])
            .expect("buffer upload");

        let mut command_buffer = command_pool
            .allocate_command_buffer()
            .expect("command buffer");
        command_buffer
            .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .expect("begin");

        // Clear the user-texture source to opaque red.
        clear_to_red(&command_buffer, &user_image);

        // The egui pass into the readback target.
        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        }];
        let begin_info = vk::RenderPassBeginInfo::default()
            .render_pass(target.render_pass().raw())
            .framebuffer(target.framebuffer().raw())
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: WIDTH,
                    height: HEIGHT,
                },
            })
            .clear_values(&clear_values);
        command_buffer.begin_render_pass(&begin_info, vk::SubpassContents::INLINE);
        renderer.render(
            &command_buffer,
            0,
            vk::Extent2D {
                width: WIDTH,
                height: HEIGHT,
            },
            pixels_per_point,
            &primitives,
        );
        command_buffer.end_render_pass();
        command_buffer.end().expect("end");

        submit_and_wait(&device, &command_buffer);
    }

    // Read back the second frame and verify.
    let pixels = read_back(&device, &command_pool, &target);
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

/// Record a clear-only render pass filling the image with opaque red.
fn clear_to_red(command_buffer: &moonfield_render::CommandBuffer, image: &OffscreenTarget) {
    let (width, height) = image.extent();
    let clear_values = [vk::ClearValue {
        color: vk::ClearColorValue {
            float32: [1.0, 0.0, 0.0, 1.0],
        },
    }];
    let begin_info = vk::RenderPassBeginInfo::default()
        .render_pass(image.render_pass().raw())
        .framebuffer(image.framebuffer().raw())
        .render_area(vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width, height },
        })
        .clear_values(&clear_values);
    command_buffer.begin_render_pass(&begin_info, vk::SubpassContents::INLINE);
    command_buffer.end_render_pass();
}

fn submit_and_wait(device: &Device, command_buffer: &moonfield_render::CommandBuffer) {
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
}

/// Copy the target's pixels into a host-visible buffer (SHADER_READ_ONLY →
/// TRANSFER_SRC → copy → back) and return them as BGRA bytes.
fn read_back(device: &Device, command_pool: &CommandPool, target: &OffscreenTarget) -> Vec<u8> {
    let readback = Buffer::new(
        device,
        (WIDTH * HEIGHT * 4) as u64,
        BufferUsage::COPY_DST,
        gpu_allocator::MemoryLocation::GpuToCpu,
    )
    .expect("readback buffer");
    let mut command_buffer = command_pool
        .allocate_command_buffer()
        .expect("command buffer");
    command_buffer
        .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .expect("begin");
    let subresource = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);
    let to_transfer = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_READ)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(target.image())
        .subresource_range(subresource);
    command_buffer.pipeline_barrier(
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::PipelineStageFlags::TRANSFER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[to_transfer],
    );
    let region = vk::BufferImageCopy::default()
        .image_subresource(
            vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .mip_level(0)
                .base_array_layer(0)
                .layer_count(1),
        )
        .image_extent(vk::Extent3D {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
        });
    // SAFETY: the target is in TRANSFER_SRC_OPTIMAL and the buffer is large
    // enough for the whole image.
    unsafe {
        device.raw().cmd_copy_image_to_buffer(
            command_buffer.raw(),
            target.image(),
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            readback.raw(),
            std::slice::from_ref(&region),
        );
    }
    let back = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(target.image())
        .subresource_range(subresource);
    command_buffer.pipeline_barrier(
        vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[back],
    );
    command_buffer.end().expect("end");
    submit_and_wait(device, &command_buffer);

    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    readback.read(&mut pixels).expect("readback");
    pixels
}

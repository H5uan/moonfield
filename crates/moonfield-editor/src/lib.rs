//! Moonfield editor plugin.
//!
//! Provides [`EditorPlugin`], a Bevy-style plugin that renders an egui-based
//! editor UI into the window owned by [`moonfield_winit::WinitPlugin`].
//! Unlike the previous design, the editor no longer owns the winit event loop
//! or the window — it registers a render-phase system in the `Render` schedule
//! via [`App::add_systems`](moonfield_app::App::add_systems) and draws
//! into the same swapchain every frame, mirroring how `bevy_egui` layers on
//! `bevy_winit` rather than replacing it.
//!
//! Composition: add `WinitPlugin` first (it owns the window + event loop,
//! spawns the primary window entity with its `Window` /
//! `RawHandleWrapper` components, and registers [`WinitWindow`],
//! [`InputState`], [`WindowControl`], the raw-event message channel), plus
//! `RenderPlugin` (it creates the shared [`RenderDevice`] world resource —
//! the Vulkan instance + logical device singletons), then `EditorPlugin`.
//! The editor reads those resources and lazily builds its window-bound
//! Vulkan objects (surface, swapchain, frame loop) + egui state on the
//! first render tick, once the window actually exists.

pub mod egui_vk;
mod registry;
mod scene_io;
mod ui;
mod viewport;

use moonfield_app::prelude::{Render, World};
use moonfield_app::{App, Plugin};
use moonfield_ecs::{MessageCursor, Messages};
use moonfield_log::error;
use moonfield_render::{RenderDevice, WindowRenderer};
use moonfield_renderer::splat::cloud::SplatCloud;
use moonfield_window::WindowControl;
use moonfield_winit::WinitWindow;
use ui::{Tab, TabContext};
use viewport::Viewport;

use ash::vk;
use std::sync::Arc;
use winit::event::WindowEvent;

/// Plugin that registers the editor render system.
///
/// The editor does not own the event loop or the window — it composes on top
/// of [`moonfield_winit::WinitPlugin`], which must be added first. Each frame
/// the winit backend calls `App::render`, which drives the editor's render
/// system to build the egui UI and record it (plus the viewport scene) into
/// the window's swapchain.
pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn name(&self) -> &str {
        "moonfield_editor::EditorPlugin"
    }

    fn build(&self, app: &mut App) {
        // The editor state is built lazily on the first render tick, once the
        // windowing backend has created the window and registered
        // `WinitWindow`.
        app.insert_resource(EditorStateSlot::default());
        app.insert_resource(registry::InspectorRegistry::with_engine_types());
        // The splat asset store: PLY files loaded through the editor land
        // here, entities reference them via SplatCloudHandle components.
        app.insert_resource(moonfield_asset::Assets::<SplatCloud>::default());
        // Synchronous asset loading (PLY → SplatCloud, path-deduped) and the
        // scene registry behind the hierarchy panel's Save/Load buttons.
        app.insert_resource(scene_io::editor_asset_server());
        app.insert_resource(scene_io::editor_scene_registry());
        app.add_systems(Render, editor_render);
    }
}

/// Lazily-initialized editor state, stored as a world resource.
///
/// `None` until the first render tick after the window exists. The blanket
/// `Resource` impl in `moonfield-ecs` covers this (it is `Send + Sync +
/// 'static` once `EditorState` is).
#[derive(Default)]
struct EditorStateSlot(Option<EditorState>);

/// All per-window editor state.
///
/// Fields are ordered for Vulkan-safe destruction (first declared drops
/// first): the egui renderer and viewport destroy resources through the
/// device, so they precede the window renderer. The device itself is *shared*
/// (the world's [`RenderDevice`] resource, held by the window renderer
/// through `Arc`s) and outlives this struct by refcounting. `Drop` waits for
/// the device to go idle before any field is destroyed.
struct EditorState {
    egui_renderer: egui_vk::EguiRenderer,
    viewport: Viewport,
    window_renderer: WindowRenderer,
    egui_state: egui_winit::State,
    dock_state: egui_dock::DockState<Tab>,
    window: Arc<winit::window::Window>,
    /// Cursor over the raw-event message channel: which winit events have
    /// already been fed into egui.
    raw_event_cursor: MessageCursor<WindowEvent>,
    /// Texture ids pending destruction, ring-buffered per in-flight frame.
    free_ring: [Vec<egui::TextureId>; 2],
    frame_counter: usize,
    /// Viewport panel size in points reported by the previous frame. The
    /// offscreen target is resized against this *before* building the UI, so
    /// the current frame's draw data always references the live texture id.
    viewport_panel_points: Option<egui::Vec2>,
    /// The entity selected in the hierarchy panel, edited by the inspector.
    selection: Option<moonfield_ecs::Entity>,
    /// Hierarchy panel state: the PLY load path field and last status.
    load_state: ui::LoadSplatState,
    /// Hierarchy panel state: the scene Save/Load path field and last status.
    scene_state: ui::SceneIoState,
    /// Frames rendered, for the MOONFIELD_EDITOR_AUTO_CLOSE debug helper.
    frames_rendered: u64,
}

impl EditorState {
    /// Build the editor state from the window registered by `WinitPlugin` and
    /// the shared render device registered by `RenderPlugin`.
    fn new(world: &World) -> Result<Self, String> {
        let winit_window = world.get_resource::<WinitWindow>().ok_or_else(|| {
            "WinitWindow resource missing — add WinitPlugin before EditorPlugin".to_string()
        })?;
        let window = winit_window.0.clone();

        let render_device = world.get_resource::<RenderDevice>().ok_or_else(|| {
            "RenderDevice resource missing — add RenderPlugin before EditorPlugin".to_string()
        })?;

        // Debug bisect: build the viewport BEFORE the window renderer, so its
        // pipeline/target predate any swapchain state.
        let mut viewport = Viewport::new(render_device.device()).map_err(|e| e.to_string())?;

        let size = window.inner_size();
        let window_renderer =
            WindowRenderer::new(&render_device, window.as_ref(), size.width, size.height)
                .map_err(|e| e.to_string())?;

        // The swapchain uses an UNORM format, so the egui shader's gamma
        // output is written verbatim (srgb_framebuffer = false).
        let srgb_framebuffer = matches!(
            window_renderer.format().format,
            vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB
        );
        let mut egui_renderer = egui_vk::EguiRenderer::new(
            window_renderer.device(),
            window_renderer.render_pass(),
            srgb_framebuffer,
            window_renderer.frames_in_flight(),
            egui_vk::RendererOptions::default(),
        )
        .map_err(|e| format!("failed to create egui renderer: {e}"))?;

        viewport.register_texture(window_renderer.device(), &mut egui_renderer);

        let egui_state = egui_winit::State::new(
            egui::Context::default(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        Ok(Self {
            egui_renderer,
            viewport,
            window_renderer,
            egui_state,
            dock_state: ui::initial_dock_state(),
            window,
            raw_event_cursor: MessageCursor::default(),
            free_ring: [Vec::new(), Vec::new()],
            frame_counter: 0,
            viewport_panel_points: None,
            selection: None,
            load_state: ui::LoadSplatState::default(),
            scene_state: ui::SceneIoState::default(),
            frames_rendered: 0,
        })
    }
}

impl Drop for EditorState {
    fn drop(&mut self) {
        // SAFETY: best-effort wait so no resource is destroyed while the GPU
        // still uses it.
        unsafe {
            let _ = self.window_renderer.device().raw().device_wait_idle();
        }
    }
}

/// Editor render system: drives egui input, builds the UI, records the
/// viewport scene + UI passes, and presents the swapchain frame.
fn editor_render(world: &mut World) {
    // Lazily build the editor state once the window exists.
    let needs_init = world
        .get_resource::<EditorStateSlot>()
        .map(|slot| slot.0.is_none())
        .unwrap_or(true);
    if needs_init {
        let state = match EditorState::new(world) {
            Ok(s) => s,
            Err(e) => {
                // The window or the shared render device may not exist yet on
                // the very first ticks (e.g. before `resumed`, or on a machine
                // without a Vulkan driver). Stay quiet and retry next frame.
                let waiting_on_prerequisites = e.contains("WinitWindow resource missing")
                    || e.contains("RenderDevice resource missing");
                if !waiting_on_prerequisites {
                    error!("failed to build editor state: {e}");
                }
                return;
            }
        };
        let mut slot = world
            .get_resource_mut::<EditorStateSlot>()
            .expect("EditorStateSlot was just checked");
        slot.0 = Some(state);
        return; // Render starts on the next tick — once init succeeds, give
                // the winit backend a clean frame boundary before recording.
    }

    // Take the state out of its slot so panels can get `&mut World`
    // (inspector edits) and the scene pass can query the world freely.
    let mut state = {
        let mut slot = world
            .get_resource_mut::<EditorStateSlot>()
            .expect("EditorStateSlot registered in build");
        let Some(state) = slot.0.take() else {
            return;
        };
        state
    };

    // Drain new raw window events into egui before building the UI. The
    // editor keeps its own cursor over the message channel (it is an
    // exclusive render system, not a MessageReader param).
    if let Some(messages) = world.get_resource::<Messages<WindowEvent>>() {
        for event in state.raw_event_cursor.read(&messages) {
            let _ = state.egui_state.on_window_event(&state.window, event);
        }
    }

    if let Err(e) = render_frame(world, &mut state) {
        error!("failed to render editor frame: {e}");
    }

    // Debug helper: MOONFIELD_EDITOR_AUTO_CLOSE=<frames> signals exit via the
    // shared WindowControl after N rendered frames, so shutdown paths can be
    // exercised from CI without manually closing the window.
    if let Ok(frames) = std::env::var("MOONFIELD_EDITOR_AUTO_CLOSE") {
        if let Ok(limit) = frames.parse::<u64>() {
            state.frames_rendered = state.frames_rendered.saturating_add(1);
            if state.frames_rendered >= limit {
                if let Some(ctrl) = world.get_resource::<WindowControl>() {
                    ctrl.request_exit();
                }
            }
        }
    }

    // Put the state back for the next frame.
    world
        .get_resource_mut::<EditorStateSlot>()
        .expect("EditorStateSlot registered in build")
        .0 = Some(state);
}

fn render_frame(world: &mut World, state: &mut EditorState) -> Result<(), String> {
    let size = state.window.inner_size();
    if size.width == 0 || size.height == 0 {
        return Ok(()); // minimized
    }

    let extent = state.window_renderer.extent();
    if state.window_renderer.needs_recreate()
        || extent.width != size.width
        || extent.height != size.height
    {
        state
            .window_renderer
            .recreate(size.width, size.height)
            .map_err(|e| e.to_string())?;
    }

    // — Resize the viewport target to match its panel (in physical pixels) —
    //
    // Uses the panel size reported by the *previous* frame so the texture id
    // referenced by this frame's UI is registered before the UI is built.
    if let Some(panel_size) = state.viewport_panel_points {
        let pixels_per_point =
            egui_winit::pixels_per_point(state.egui_state.egui_ctx(), &state.window);
        let width = (panel_size.x * pixels_per_point).round().max(1.0) as u32;
        let height = (panel_size.y * pixels_per_point).round().max(1.0) as u32;
        if (width, height) != state.viewport.extent() {
            state
                .viewport
                .resize(state.window_renderer.device(), width, height)
                .map_err(|e| e.to_string())?;
            state
                .viewport
                .register_texture(state.window_renderer.device(), &mut state.egui_renderer);
        }
    }

    // — egui: build the UI —
    let egui_ctx = state.egui_state.egui_ctx().clone();
    let raw_input = state.egui_state.take_egui_input(&state.window);
    let mut tab_context = TabContext {
        world: &mut *world,
        selection: &mut state.selection,
        load_state: &mut state.load_state,
        scene_state: &mut state.scene_state,
        viewport_texture: state.viewport.texture_id(),
        viewport_size_points: None,
    };
    let full_output = egui_ctx.run(raw_input, |ctx| {
        ui::show(ctx, &mut state.dock_state, &mut tab_context);
    });
    let egui::FullOutput {
        platform_output,
        textures_delta,
        shapes,
        pixels_per_point,
        ..
    } = full_output;
    state
        .egui_state
        .handle_platform_output(&state.window, platform_output);
    state.viewport_panel_points = tab_context.viewport_size_points;

    // — Begin the swapchain frame —
    if !state
        .window_renderer
        .begin_frame()
        .map_err(|e| e.to_string())?
    {
        return Ok(()); // swapchain out of date; recreated on the next frame
    }

    // From here on a frame is in progress: any error must go through
    // `finish_frame` so the frame state (fence, semaphores, acquired image)
    // stays consistent.
    let result = record_frame(
        world,
        state,
        &egui_ctx,
        shapes,
        &textures_delta,
        pixels_per_point,
    );
    finish_frame(state, result)?;

    // Debug seam: MOONFIELD_EDITOR_DUMP_VIEWPORT=<frame> dumps the viewport
    // target's pixels (raw BGRA) to target/tmp/viewport_dump_<w>x<h>.raw
    // once that many frames have been rendered.
    if std::env::var("MOONFIELD_EDITOR_DUMP_VIEWPORT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        == Some(state.frames_rendered + 1)
    {
        if let Err(e) = dump_viewport_target(state) {
            error!("failed to dump viewport target: {e}");
        }
    }

    // Queue this frame's texture frees; they become safe to destroy once the
    // fence for this frame slot passes again.
    let ring_index = state.frame_counter % state.free_ring.len();
    state.free_ring[ring_index].extend(textures_delta.free.iter().copied());
    state.frame_counter += 1;
    Ok(())
}

/// Record the scene and UI passes. Returns whether the UI render pass was
/// left open on error, so `finish_frame` can close it before submitting.
fn record_frame(
    world: &World,
    state: &mut EditorState,
    egui_ctx: &egui::Context,
    shapes: Vec<egui::epaint::ClippedShape>,
    textures_delta: &egui::TexturesDelta,
    pixels_per_point: f32,
) -> Result<(), String> {
    // The fence for this frame slot just passed: textures freed by egui two
    // frames ago are no longer sampled.
    let ring_index = state.frame_counter % state.free_ring.len();
    let pending = std::mem::take(&mut state.free_ring[ring_index]);
    state.egui_renderer.free_textures(&pending);

    // Upload egui-managed textures (fonts, …) before recording.
    for (id, delta) in &textures_delta.set {
        state
            .egui_renderer
            .update_texture(state.window_renderer.device(), *id, delta)?;
    }

    // Upload this frame's mesh data into the current slot's buffers.
    let primitives = egui_ctx.tessellate(shapes, pixels_per_point);
    let extent = state.window_renderer.extent();
    let screen_size_points = [
        extent.width as f32 / pixels_per_point,
        extent.height as f32 / pixels_per_point,
    ];
    let frame_slot = state.window_renderer.current_frame_index();
    state.egui_renderer.update_buffers(
        state.window_renderer.device(),
        frame_slot,
        &primitives,
        screen_size_points,
    )?;

    // — Scene pass into the viewport's offscreen target —
    if std::env::var_os("MOONFIELD_EDITOR_SCENE_ONESHOT").is_some() {
        // Debug seam: record the scene into a fresh one-shot command buffer
        // instead of the frame's, to isolate command-buffer context effects.
        let device = state.window_renderer.device();
        let pool =
            moonfield_render::CommandPool::new(device, device.queue_family_indices().graphics)
                .map_err(|e| e.to_string())?;
        let mut cmd = pool.allocate_command_buffer().map_err(|e| e.to_string())?;
        cmd.begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
            .map_err(|e| e.to_string())?;
        state.viewport.record_scene(world, &cmd);
        cmd.end().map_err(|e| e.to_string())?;
        let command_buffers = [cmd.raw()];
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
                .map_err(|e| format!("scene one-shot submit: {e:?}"))?;
            device
                .raw()
                .queue_wait_idle(device.graphics_queue())
                .map_err(|e| format!("scene one-shot wait: {e:?}"))?;
        }
    } else {
        state
            .viewport
            .record_scene(world, state.window_renderer.command_buffer());
    }

    // Debug seam: MOONFIELD_EDITOR_SKIP_UI=1 records only the scene pass.
    if std::env::var_os("MOONFIELD_EDITOR_SKIP_UI").is_some() {
        return Ok(());
    }

    // — UI pass into the swapchain image —
    let framebuffer = state.window_renderer.framebuffer().raw();
    let clear_values = [vk::ClearValue {
        color: vk::ClearColorValue {
            float32: [0.0, 0.0, 0.0, 1.0],
        },
    }];
    let begin_info = vk::RenderPassBeginInfo::default()
        .render_pass(state.window_renderer.render_pass().raw())
        .framebuffer(framebuffer)
        .render_area(vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        })
        .clear_values(&clear_values);
    let command_buffer = state.window_renderer.command_buffer();
    command_buffer.begin_render_pass(&begin_info, vk::SubpassContents::INLINE);
    state.egui_renderer.render(
        command_buffer,
        frame_slot,
        extent,
        pixels_per_point,
        &primitives,
    );
    command_buffer.end_render_pass();
    Ok(())
}

/// Complete the in-progress frame regardless of recording errors, so the
/// renderer never gets stuck with a dangling acquired image.
fn finish_frame(state: &mut EditorState, result: Result<(), String>) -> Result<(), String> {
    state
        .window_renderer
        .end_frame()
        .map_err(|e| e.to_string())?;
    result
}

/// Copy the viewport's offscreen target into a host buffer and write the raw
/// BGRA pixels to `target/tmp/viewport_dump_<w>x<h>.raw`. Debug seam for the
/// `MOONFIELD_EDITOR_DUMP_VIEWPORT` env var.
fn dump_viewport_target(state: &EditorState) -> Result<(), String> {
    use moonfield_render::{BufferUsage, CommandPool};

    let device = state.window_renderer.device();
    let target = state.viewport.target();
    let (width, height) = target.extent();
    let readback = moonfield_render::Buffer::new(
        device,
        (width * height * 4) as u64,
        BufferUsage::COPY_DST,
        gpu_allocator::MemoryLocation::GpuToCpu,
    )
    .map_err(|e| e.to_string())?;

    let command_pool = CommandPool::new(device, device.queue_family_indices().graphics)
        .map_err(|e| e.to_string())?;
    let mut command_buffer = command_pool
        .allocate_command_buffer()
        .map_err(|e| e.to_string())?;
    command_buffer
        .begin(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
        .map_err(|e| e.to_string())?;
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
            width,
            height,
            depth: 1,
        });
    // SAFETY: the target is in TRANSFER_SRC_OPTIMAL and the buffer fits it.
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
    command_buffer.end().map_err(|e| e.to_string())?;

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
            .map_err(|e| format!("failed to submit viewport dump: {e:?}"))?;
        device
            .raw()
            .queue_wait_idle(device.graphics_queue())
            .map_err(|e| format!("failed to wait for viewport dump: {e:?}"))?;
    }

    let mut pixels = vec![0u8; (width * height * 4) as usize];
    readback.read(&mut pixels).map_err(|e| e.to_string())?;
    std::fs::create_dir_all("target/tmp").map_err(|e| e.to_string())?;
    std::fs::write(
        format!("target/tmp/viewport_dump_{width}x{height}.raw"),
        pixels,
    )
    .map_err(|e| e.to_string())
}

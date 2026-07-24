//! Moonfield editor plugin.
//!
//! Provides [`EditorPlugin`], a Bevy-style plugin that renders an egui-based
//! editor UI into the window owned by [`moonfield_winit::WinitPlugin`].
//! Unlike the previous design, the editor no longer owns the winit event loop
//! or the window — it registers a render-phase system via
//! [`App::add_render_system`](moonfield_app::App::add_render_system) and draws
//! into the same swapchain every frame, mirroring how `bevy_egui` layers on
//! `bevy_winit` rather than replacing it.
//!
//! Composition: add `WinitPlugin` first (it owns the window + event loop and
//! registers [`WinitWindow`], [`RawHandleWrapper`], [`InputState`],
//! [`WindowControl`], [`RawWindowEvents`]), then `EditorPlugin`. The editor
//! reads those resources and lazily builds its Vulkan + egui state on the
//! first render tick, once the window actually exists.

mod ui;
mod viewport;

use moonfield_app::prelude::World;
use moonfield_app::{App, Plugin};
use moonfield_log::error;
use moonfield_render::WindowRenderer;
use moonfield_window::WindowControl;
use moonfield_winit::{RawWindowEvents, WinitWindow};
use ui::{Tab, TabContext};
use viewport::Viewport;

use ash::vk;
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use std::sync::{Arc, Mutex};
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
        // `WinitWindow` / `RawHandleWrapper`.
        app.insert_resource(EditorStateSlot::default());
        app.add_render_system(editor_render);
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
/// device, so they precede the window renderer that owns it. The allocator
/// also precedes it — gpu-allocator frees its cached memory blocks via
/// `vkFreeMemory` on drop, which requires a live device. `Drop` waits for
/// the device to go idle before any field is destroyed.
struct EditorState {
    egui_renderer: egui_ash_renderer::Renderer,
    viewport: Viewport,
    upload_pool: moonfield_render::CommandPool,
    /// Held to keep the allocator alive; the egui renderer and viewport
    /// share clones of it.
    #[allow(dead_code)]
    allocator: Arc<Mutex<Allocator>>,
    window_renderer: WindowRenderer,
    egui_state: egui_winit::State,
    dock_state: egui_dock::DockState<Tab>,
    window: Arc<winit::window::Window>,
    /// Texture ids pending destruction, ring-buffered per in-flight frame.
    free_ring: [Vec<egui::TextureId>; 2],
    frame_counter: usize,
    /// Viewport panel size in points reported by the previous frame. The
    /// offscreen target is resized against this *before* building the UI, so
    /// the current frame's draw data always references the live texture id.
    viewport_panel_points: Option<egui::Vec2>,
    /// Frames rendered, for the MOONFIELD_EDITOR_AUTO_CLOSE debug helper.
    frames_rendered: u64,
}

impl EditorState {
    /// Build the editor state from the window registered by `WinitPlugin`.
    fn new(world: &World) -> Result<Self, String> {
        let winit_window = world.get_resource::<WinitWindow>().ok_or_else(|| {
            "WinitWindow resource missing — add WinitPlugin before EditorPlugin".to_string()
        })?;
        let window = winit_window.0.clone();

        let size = window.inner_size();
        let window_renderer = WindowRenderer::new(window.as_ref(), size.width, size.height)
            .map_err(|e| e.to_string())?;

        let allocator = Arc::new(Mutex::new(
            Allocator::new(&AllocatorCreateDesc {
                instance: window_renderer.instance().raw().clone(),
                device: window_renderer.device().raw().clone(),
                physical_device: window_renderer.device().physical_device(),
                debug_settings: Default::default(),
                buffer_device_address: false,
                allocation_sizes: Default::default(),
            })
            .map_err(|e| format!("failed to create GPU allocator: {e}"))?,
        ));

        let mut egui_renderer = egui_ash_renderer::Renderer::with_gpu_allocator(
            allocator.clone(),
            window_renderer.device().raw().clone(),
            window_renderer.render_pass().raw(),
            egui_ash_renderer::Options {
                in_flight_frames: 2,
                enable_depth_test: false,
                enable_depth_write: false,
                // The swapchain uses an UNORM format, so the egui shader
                // outputs sRGB-encoded colors itself.
                srgb_framebuffer: false,
            },
        )
        .map_err(|e| format!("failed to create egui renderer: {e}"))?;

        let mut viewport = Viewport::new(
            window_renderer.instance(),
            window_renderer.device(),
            allocator.clone(),
        )
        .map_err(|e| e.to_string())?;
        viewport.register_texture(&mut egui_renderer);

        let upload_pool = moonfield_render::CommandPool::new(
            window_renderer.device(),
            window_renderer.device().queue_family_indices().graphics,
        )
        .map_err(|e| e.to_string())?;

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
            upload_pool,
            window_renderer,
            allocator,
            egui_state,
            dock_state: ui::initial_dock_state(),
            window,
            free_ring: [Vec::new(), Vec::new()],
            frame_counter: 0,
            viewport_panel_points: None,
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
                // The window may not exist yet on the very first ticks
                // (e.g. before `resumed`). Stay quiet and retry next frame.
                if !e.contains("WinitWindow resource missing") {
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

    let mut slot = world
        .get_resource_mut::<EditorStateSlot>()
        .expect("EditorStateSlot registered in build");
    let Some(state) = slot.0.as_mut() else {
        return;
    };

    // Drain raw window events into egui before building the UI.
    let raw_events: Vec<WindowEvent> = world
        .get_resource::<RawWindowEvents>()
        .map(|r| r.events().to_vec())
        .unwrap_or_default();
    for event in &raw_events {
        let _ = state.egui_state.on_window_event(&state.window, event);
    }

    if let Err(e) = render_frame(state) {
        error!("failed to render editor frame: {e}");
    }

    // Debug helper: MOONFIELD_EDITOR_AUTO_CLOSE=<frames> signals exit via the
    // shared WindowControl after N rendered frames, so shutdown paths can be
    // exercised from scripts/CI without manually closing the window.
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
}

fn render_frame(state: &mut EditorState) -> Result<(), String> {
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
            state.viewport.register_texture(&mut state.egui_renderer);
        }
    }

    // — egui: build the UI —
    let egui_ctx = state.egui_state.egui_ctx().clone();
    let raw_input = state.egui_state.take_egui_input(&state.window);
    let mut tab_context = TabContext {
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
    let result = record_frame(state, &egui_ctx, shapes, &textures_delta, pixels_per_point);
    finish_frame(state, result)?;

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
    state: &mut EditorState,
    egui_ctx: &egui::Context,
    shapes: Vec<egui::epaint::ClippedShape>,
    textures_delta: &egui::TexturesDelta,
    pixels_per_point: f32,
) -> Result<(), RecordError> {
    // The fence for this frame slot just passed: textures freed by egui two
    // frames ago are no longer sampled.
    let ring_index = state.frame_counter % state.free_ring.len();
    let pending = std::mem::take(&mut state.free_ring[ring_index]);
    if !pending.is_empty() {
        state
            .egui_renderer
            .free_textures(&pending)
            .map_err(|e| RecordError::BeforePass(e.to_string()))?;
    }

    // Upload egui-managed textures (fonts, …) before recording.
    state
        .egui_renderer
        .set_textures(
            state.window_renderer.device().graphics_queue(),
            state.upload_pool.raw(),
            &textures_delta.set,
        )
        .map_err(|e| RecordError::BeforePass(e.to_string()))?;

    // — Scene pass into the viewport's offscreen target —
    state
        .viewport
        .record_scene(state.window_renderer.command_buffer());

    // — UI pass into the swapchain image —
    let primitives = egui_ctx.tessellate(shapes, pixels_per_point);
    let extent = state.window_renderer.extent();
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
    state
        .egui_renderer
        .cmd_draw(command_buffer.raw(), extent, pixels_per_point, &primitives)
        .map_err(|e| RecordError::InsidePass(e.to_string()))?;
    command_buffer.end_render_pass();
    Ok(())
}

/// Errors during frame recording, tracking whether the UI render pass is
/// still open and needs closing before the command buffer can be ended.
enum RecordError {
    BeforePass(String),
    InsidePass(String),
}

/// Complete the in-progress frame regardless of recording errors, so the
/// renderer never gets stuck with a dangling acquired image.
fn finish_frame(state: &mut EditorState, result: Result<(), RecordError>) -> Result<(), String> {
    let (ui_pass_open, recording_error) = match result {
        Ok(()) => (false, None),
        Err(RecordError::BeforePass(e)) => (false, Some(e)),
        Err(RecordError::InsidePass(e)) => (true, Some(e)),
    };
    if ui_pass_open {
        state.window_renderer.command_buffer().end_render_pass();
    }
    state
        .window_renderer
        .end_frame()
        .map_err(|e| e.to_string())?;
    match recording_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

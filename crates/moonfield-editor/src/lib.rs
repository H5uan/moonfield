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
    egui_renderer: egui_ash_renderer::Renderer,
    viewport: Viewport,
    upload_pool: moonfield_render::CommandPool,
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

        let size = window.inner_size();
        let window_renderer =
            WindowRenderer::new(&render_device, window.as_ref(), size.width, size.height)
                .map_err(|e| e.to_string())?;

        // The egui renderer needs the same GPU allocator the device owns.
        let allocator = window_renderer.device().allocator().clone();

        let mut egui_renderer = egui_ash_renderer::Renderer::with_gpu_allocator(
            allocator,
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

        let mut viewport = Viewport::new(window_renderer.device()).map_err(|e| e.to_string())?;
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
            egui_state,
            dock_state: ui::initial_dock_state(),
            window,
            raw_event_cursor: MessageCursor::default(),
            free_ring: [Vec::new(), Vec::new()],
            frame_counter: 0,
            viewport_panel_points: None,
            selection: None,
            load_state: ui::LoadSplatState::default(),
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
            state.viewport.register_texture(&mut state.egui_renderer);
        }
    }

    // — egui: build the UI —
    let egui_ctx = state.egui_state.egui_ctx().clone();
    let raw_input = state.egui_state.take_egui_input(&state.window);
    let mut tab_context = TabContext {
        world: &mut *world,
        selection: &mut state.selection,
        load_state: &mut state.load_state,
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
        .record_scene(world, state.window_renderer.command_buffer());

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

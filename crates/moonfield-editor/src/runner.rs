//! Editor event loop: winit + egui + the windowed Vulkan renderer.
//!
//! [`EditorPlugin`] replaces the app's runner with an editor-specific winit
//! event loop that drives egui on top of [`WindowRenderer`]. The frame flow
//! is: `app.update()` → egui UI → scene pass into the viewport's offscreen
//! target → egui pass into the swapchain → present.

use crate::ui::{self, Tab, TabContext};
use crate::viewport::Viewport;
use ash::vk;
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use moonfield_app::{App, Plugin};
use moonfield_log::error;
use moonfield_render::{CommandPool, WindowRenderer};
use std::sync::{Arc, Mutex};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

/// Plugin that installs the editor event loop as the app's runner.
pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn name(&self) -> &str {
        "moonfield_editor::EditorPlugin"
    }

    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        app.set_runner(editor_runner);
    }
}

/// The editor runner: creates the window, the Vulkan renderer and the egui
/// integration, then drives everything from the winit event loop.
pub fn editor_runner(app: &mut App) {
    let event_loop = EventLoop::new().expect("failed to create winit event loop");
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut handler = EditorHandler {
        app,
        state: None,
        frames_rendered: 0,
    };
    if let Err(e) = event_loop.run_app(&mut handler) {
        error!("editor event loop exited with error: {e}");
    }
}

struct EditorHandler<'a> {
    app: &'a mut App,
    state: Option<EditorState>,
    /// Frames rendered, for the MOONFIELD_EDITOR_AUTO_CLOSE debug helper.
    frames_rendered: u64,
}

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
    upload_pool: CommandPool,
    /// Held to keep the allocator alive; the egui renderer and viewport
    /// share clones of it.
    #[allow(dead_code)]
    allocator: Arc<Mutex<Allocator>>,
    window_renderer: WindowRenderer,
    egui_state: egui_winit::State,
    dock_state: egui_dock::DockState<Tab>,
    window: Arc<Window>,
    /// Texture ids pending destruction, ring-buffered per in-flight frame.
    free_ring: [Vec<egui::TextureId>; 2],
    frame_counter: usize,
    /// Viewport panel size in points reported by the previous frame. The
    /// offscreen target is resized against this *before* building the UI, so
    /// the current frame's draw data always references the live texture id.
    viewport_panel_points: Option<egui::Vec2>,
}

impl EditorState {
    fn new(event_loop: &ActiveEventLoop) -> Self {
        let attrs = WindowAttributes::default()
            .with_title("Moonfield Editor")
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create editor window"),
        );

        let size = window.inner_size();
        let window_renderer = WindowRenderer::new(window.as_ref(), size.width, size.height)
            .expect("failed to create window renderer");

        let allocator = Arc::new(Mutex::new(
            Allocator::new(&AllocatorCreateDesc {
                instance: window_renderer.instance().raw().clone(),
                device: window_renderer.device().raw().clone(),
                physical_device: window_renderer.device().physical_device(),
                debug_settings: Default::default(),
                buffer_device_address: false,
                allocation_sizes: Default::default(),
            })
            .expect("failed to create GPU allocator"),
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
        .expect("failed to create egui renderer");

        let mut viewport = Viewport::new(
            window_renderer.instance(),
            window_renderer.device(),
            allocator.clone(),
        )
        .expect("failed to create viewport");
        viewport.register_texture(&mut egui_renderer);

        let upload_pool = CommandPool::new(
            window_renderer.device(),
            window_renderer.device().queue_family_indices().graphics,
        )
        .expect("failed to create upload command pool");

        let egui_state = egui_winit::State::new(
            egui::Context::default(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        Self {
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
        }
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

impl ApplicationHandler for EditorHandler<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            self.state = Some(EditorState::new(event_loop));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = self.state.as_mut() else {
            return;
        };

        // The editor consumes every event for egui; gameplay input routing
        // (only when the viewport is focused) comes with script integration.
        let _ = state.egui_state.on_window_event(&state.window, &event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(e) = render_frame(self.app, state) {
                    error!("failed to render editor frame: {e}");
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.app.update();
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
        // Debug helper: MOONFIELD_EDITOR_AUTO_CLOSE=<frames> exits the loop
        // after N rendered frames, so shutdown paths can be exercised from
        // scripts/CI without manually closing the window.
        if let Ok(frames) = std::env::var("MOONFIELD_EDITOR_AUTO_CLOSE") {
            if let Ok(limit) = frames.parse::<u64>() {
                self.frames_rendered = self.frames_rendered.saturating_add(1);
                if self.frames_rendered >= limit {
                    event_loop.exit();
                }
            }
        }
    }
}

fn render_frame(_app: &mut App, state: &mut EditorState) -> Result<(), String> {
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

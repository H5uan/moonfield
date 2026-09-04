//! Moonfield editor plugin.
//!
//! Provides [`EditorPlugin`], a Bevy-style plugin that renders an egui-based
//! editor UI into the window owned by [`moonfield_winit::WinitPlugin`].
//! The editor does not own the winit event loop or window — and, bevy-style,
//! there is no editor "renderer" object either. `PreRender` builds the UI in
//! the main world, [`extract_editor_frame`] moves the prepared frame into the
//! render world, and three ordered `Render` systems present it:
//! [`prepare_egui_frame`] (texture uploads, viewport binding, tessellation,
//! buffer uploads), [`egui_pass`] (records the UI pass into the acquired
//! swapchain image), and [`editor_frame_done`] (feeds results back to the
//! main world). The window frame loop (acquire/submit) belongs to
//! `moonfield_render_core`'s window systems; the viewport scene pass to
//! `moonfield_render_feature`.
//!
//! Composition: add `WinitPlugin` first (it owns the window + event loop,
//! spawns the primary window entity with its `Window` /
//! `RawHandleWrapper` components, and registers [`WinitWindow`],
//! [`InputState`], [`WindowControl`], the raw-event message channel), plus
//! `RenderPlugin` (it creates the render-world [`RenderDevice`] resource and
//! drives the window frame loop) and `RenderFeaturePlugin` (asset stores, the
//! scene pass, view targets), then `EditorPlugin`. CPU state and Vulkan state
//! initialize independently once their world contains the required resources.

pub mod egui_vk;
mod interaction;
mod registry;
mod scene_io;
mod theme;
mod ui;

pub use scene_io::{editor_asset_server, load_asset};

use moonfield_app::prelude::{IntoSystemConfigs, PreRender, Render, World};
use moonfield_app::{App, Plugin};
use moonfield_camera::{PrimaryCamera, RenderTarget};
use moonfield_ecs::{MessageCursor, Messages, ensure_global_transforms};
use moonfield_log::error;
use moonfield_render_core::{MAX_FRAMES_IN_FLIGHT, ViewTargets, WindowFrameDemand, WindowSurfaces};
use moonfield_render_feature::core_3d::pass::RenderTargetSizes;
use moonfield_rhi::{
    AttachmentLayout, ClearValue, LoadOp, Rect2d, RenderAttachment, RenderDevice, RenderPassDesc,
    SamplerHandle, StoreOp, TextureHandle,
};
use moonfield_window::WindowControl;
use moonfield_winit::WinitWindow;
use ui::{Tab, TabContext};

use std::sync::{Arc, Mutex, MutexGuard};
use winit::event::WindowEvent;

/// Plugin that registers the editor's prepare/extract/render systems.
///
/// The editor does not own the event loop or the window — it composes on top
/// of [`moonfield_winit::WinitPlugin`], which must be added first. Each frame
/// the winit backend calls `App::render`, which drives `PreRender` (build the
/// egui UI), extraction (hand the frame to the render world), and the
/// `Render` systems (upload egui data, record the UI pass into the window
/// frame between `moonfield_render_core`'s acquire/submit systems, publish
/// feedback).
pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn name(&self) -> &str {
        "moonfield_editor::EditorPlugin"
    }

    fn build(&self, app: &mut App) {
        let feedback = EditorFeedbackChannel::default();
        app.insert_resource(EditorMainStateSlot::default());
        app.insert_resource(PendingEditorFrame::default());
        app.insert_resource(feedback.clone());
        app.render_world_mut().insert_resource(feedback);
        app.render_world_mut()
            .insert_resource(ViewportTexture::default());
        app.insert_resource(registry::InspectorRegistry::with_engine_types());
        // Asset stores are owned by RenderFeaturePlugin, which must be added
        // first. Loading is synchronous and path-deduped; the scene registry
        // backs the hierarchy panel's Save/Load buttons.
        app.insert_resource(scene_io::editor_asset_server());
        app.insert_resource(scene_io::editor_scene_registry());
        app.add_extract_system(extract_editor_frame);
        app.add_systems(PreRender, editor_prepare.before(&ensure_global_transforms));
        app.add_render_systems(
            Render,
            (
                prepare_egui_frame
                    .after(&moonfield_render_core::acquire_window_frames)
                    .after(&moonfield_render_feature::core_3d::pass::prepare_view_targets)
                    .before(&egui_pass),
                egui_pass
                    .after(&moonfield_render_feature::core_3d::pass::main_opaque_pass_3d)
                    .before(&moonfield_render_core::submit_window_frames),
                editor_frame_done.after(&moonfield_render_core::submit_window_frames),
            ),
        );
    }
}

/// Main-world slot for the newest CPU-side editor frame, published by
/// [`editor_prepare`] and taken by [`extract_editor_frame`]. An unconsumed
/// frame merges with the next one so egui texture deltas are never dropped.
#[derive(Default)]
struct PendingEditorFrame(Option<PreparedEditorFrame>);

/// One CPU-side editor frame: the egui context, shapes, texture deltas, and
/// the viewport panel's size request. In the render world the same type is
/// the resource [`extract_editor_frame`] merges into and
/// [`prepare_egui_frame`] consumes.
struct PreparedEditorFrame {
    egui_ctx: egui::Context,
    shapes: Vec<egui::epaint::ClippedShape>,
    textures_delta: egui::TexturesDelta,
    pixels_per_point: f32,
    viewport_panel_points: Option<egui::Vec2>,
}

impl Drop for PreparedEditorFrame {
    fn drop(&mut self) {
        // epaint 0.36 debug-asserts that a dropped `TexturesDelta` was fully
        // applied; a frame discarded before `prepare_egui_frame` ran still
        // carries unapplied deltas.
        self.textures_delta.clear();
    }
}

/// Merge an unconsumed frame into the newer one: the newest shapes and panel
/// state win, but texture deltas accumulate so no upload or free is lost.
fn merge_prepared_frames(
    mut stale: PreparedEditorFrame,
    mut latest: PreparedEditorFrame,
) -> PreparedEditorFrame {
    let mut textures_delta = std::mem::take(&mut stale.textures_delta);
    textures_delta.append(std::mem::take(&mut latest.textures_delta));
    latest.textures_delta = textures_delta;
    latest
}

/// Render→main feedback payload: the viewport's egui texture id and the
/// number of presented frames (drives the `MOONFIELD_EDITOR_AUTO_CLOSE`
/// debug helper).
#[derive(Clone, Copy, Default)]
struct EditorFeedback {
    viewport_texture: Option<egui::TextureId>,
    frames_rendered: u64,
}

/// The feedback channel shared between both worlds — the same `Arc` is
/// inserted into each. The payload is an `Option` so `take` can distinguish
/// "no new feedback" from a payload whose fields happen to be empty.
#[derive(Clone, Default)]
struct EditorFeedbackChannel(Arc<Mutex<Option<EditorFeedback>>>);

impl EditorFeedbackChannel {
    fn lock(&self) -> MutexGuard<'_, Option<EditorFeedback>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn publish(&self, feedback: EditorFeedback) {
        *self.lock() = Some(feedback);
    }

    fn take(&self) -> Option<EditorFeedback> {
        self.lock().take()
    }
}

/// Render-world binding of the viewport's offscreen target as an egui
/// texture. A target resize allocates new heap slots, so the registration
/// refreshes whenever the handles change.
#[derive(Default)]
struct ViewportTexture {
    id: Option<egui::TextureId>,
    /// The heap handles `id` was registered from.
    handles: Option<(TextureHandle, SamplerHandle)>,
}

/// Render-world resource: a tessellated, GPU-uploaded egui frame, produced by
/// [`prepare_egui_frame`] and consumed by [`egui_pass`] within the same tick.
struct EguiPreparedFrame {
    primitives: Vec<egui::epaint::ClippedPrimitive>,
    pixels_per_point: f32,
    frame_slot: usize,
}

#[derive(Default)]
struct EditorMainStateSlot(Option<EditorMainState>);

/// Main-world editor state. It owns only CPU-side UI and interaction data.
struct EditorMainState {
    egui_state: egui_winit::State,
    dock_state: egui_dock::DockState<Tab>,
    window: Arc<winit::window::Window>,
    /// Cursor over the raw-event message channel: which winit events have
    /// already been fed into egui.
    raw_event_cursor: MessageCursor<WindowEvent>,
    viewport_texture: Option<egui::TextureId>,
    /// The entity selected in the hierarchy panel, edited by the inspector.
    selection: Option<moonfield_ecs::Entity>,
    /// Content panel state: the asset load path field and last status.
    load_state: ui::LoadAssetState,
    /// Content panel state: the scene Save/Load path field and last status.
    scene_state: ui::SceneIoState,
    /// The editor-owned orbit camera driving the `PrimaryCamera` entity;
    /// `None` until the first frame the viewport panel sees the camera.
    camera: Option<interaction::OrbitCamera>,
    /// The active gizmo operation (W/E/R in the viewport).
    gizmo_mode: interaction::GizmoMode,
    /// The in-progress gizmo drag, if any.
    gizmo_drag: Option<interaction::GizmoDrag>,
    /// Render-world completion count, for the auto-close debug helper.
    frames_rendered: u64,
}

impl EditorMainState {
    fn new(world: &World) -> Result<Self, String> {
        let winit_window = world.get_resource::<WinitWindow>().ok_or_else(|| {
            "WinitWindow resource missing — add WinitPlugin before EditorPlugin".to_string()
        })?;
        let window = winit_window.0.clone();

        let egui_ctx = egui::Context::default();
        theme::install(&egui_ctx);
        let egui_state = egui_winit::State::new(
            egui_ctx,
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        Ok(Self {
            egui_state,
            dock_state: ui::initial_dock_state(),
            window,
            raw_event_cursor: MessageCursor::default(),
            viewport_texture: None,
            selection: None,
            load_state: ui::LoadAssetState::default(),
            scene_state: ui::SceneIoState::default(),
            camera: None,
            gizmo_mode: interaction::GizmoMode::Translate,
            gizmo_drag: None,
            frames_rendered: 0,
        })
    }
}

/// Extract system: move the pending editor frame into the render world
/// (merging into an unconsumed one so texture deltas survive), report the
/// viewport panel's physical size to [`RenderTargetSizes`], and set
/// [`WindowFrameDemand`] so window frames are only acquired when there is UI
/// content to present.
fn extract_editor_frame(main_world: &World, render_world: &mut World) {
    let frame = main_world
        .get_resource_mut::<PendingEditorFrame>()
        .and_then(|mut pending| pending.0.take());
    render_world.insert_resource(WindowFrameDemand(frame.is_some()));
    let Some(frame) = frame else {
        return;
    };

    if let Some(panel_size) = frame.viewport_panel_points {
        let width = (panel_size.x * frame.pixels_per_point).round().max(1.0) as u32;
        let height = (panel_size.y * frame.pixels_per_point).round().max(1.0) as u32;
        if !render_world.contains_resource::<RenderTargetSizes>() {
            render_world.insert_resource(RenderTargetSizes::default());
        }
        render_world
            .get_resource_mut::<RenderTargetSizes>()
            .expect("RenderTargetSizes was just ensured")
            .0
            .insert(RenderTarget::Viewport, (width, height));
    }

    let frame = match render_world.remove_resource::<PreparedEditorFrame>() {
        Some(stale) => merge_prepared_frames(stale, frame),
        None => frame,
    };
    render_world.insert_resource(frame);
}

/// Main-world `PreRender` system: handles input, builds the UI, and stages
/// the newest CPU frame for extraction.
fn editor_prepare(world: &mut World) {
    let needs_init = world
        .get_resource::<EditorMainStateSlot>()
        .map(|slot| slot.0.is_none())
        .unwrap_or(true);
    if needs_init {
        let state = match EditorMainState::new(world) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut slot = world
            .get_resource_mut::<EditorMainStateSlot>()
            .expect("EditorMainStateSlot was just checked");
        slot.0 = Some(state);
    }

    let feedback = world
        .get_resource::<EditorFeedbackChannel>()
        .map(|channel| (*channel).clone())
        .expect("EditorFeedbackChannel registered in build");
    let mut state = {
        let mut slot = world
            .get_resource_mut::<EditorMainStateSlot>()
            .expect("EditorMainStateSlot registered in build");
        let Some(state) = slot.0.take() else {
            return;
        };
        state
    };

    // Take semantics: without new feedback the last-known values stay.
    if let Some(feedback) = feedback.take() {
        state.viewport_texture = feedback.viewport_texture;
        state.frames_rendered = feedback.frames_rendered;
    }

    if let Some(messages) = world.get_resource::<Messages<WindowEvent>>() {
        for event in state.raw_event_cursor.read(&messages) {
            let _ = state.egui_state.on_window_event(&state.window, event);
        }
    }

    let egui_ctx = state.egui_state.egui_ctx().clone();
    let raw_input = state.egui_state.take_egui_input(&state.window);
    let (full_output, viewport_panel_points) = {
        let mut tab_context = TabContext {
            world: &mut *world,
            selection: &mut state.selection,
            load_state: &mut state.load_state,
            scene_state: &mut state.scene_state,
            viewport_texture: state.viewport_texture,
            viewport_size_points: None,
            camera: &mut state.camera,
            gizmo_mode: &mut state.gizmo_mode,
            gizmo_drag: &mut state.gizmo_drag,
        };
        let full_output = egui_ctx.run_ui(raw_input, |ui| {
            ui::show(ui, &mut state.dock_state, &mut tab_context);
        });
        (full_output, tab_context.viewport_size_points)
    };
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
    if let Some(camera) = &state.camera {
        apply_orbit_camera(world, camera);
    }

    let frame = PreparedEditorFrame {
        egui_ctx,
        shapes,
        textures_delta,
        pixels_per_point,
        viewport_panel_points,
    };
    let mut pending = world
        .get_resource_mut::<PendingEditorFrame>()
        .expect("PendingEditorFrame registered in build");
    pending.0 = Some(match pending.0.take() {
        Some(stale) => merge_prepared_frames(stale, frame),
        None => frame,
    });
    drop(pending);

    if let Ok(frames) = std::env::var("MOONFIELD_EDITOR_AUTO_CLOSE")
        && let Ok(limit) = frames.parse::<u64>()
        && state.frames_rendered >= limit
        && let Some(ctrl) = world.get_resource::<WindowControl>()
    {
        ctrl.request_exit();
    }

    world
        .get_resource_mut::<EditorMainStateSlot>()
        .expect("EditorMainStateSlot registered in build")
        .0 = Some(state);
}

/// Write the editor orbit camera's pose into the primary camera entity's
/// `Transform`. A side effect of the editor owning the viewport camera:
/// editing the camera's `Transform` in the inspector is overwritten here.
fn apply_orbit_camera(world: &mut World, camera: &interaction::OrbitCamera) {
    let mut target = None;
    for (entity, _) in world.query::<&PrimaryCamera>() {
        if world
            .get_component::<moonfield_math::Transform>(entity)
            .is_some()
        {
            target = Some(entity);
            break;
        }
    }
    if let Some(entity) = target
        && let Some(mut transform) = world.get_component_mut::<moonfield_math::Transform>(entity)
    {
        *transform = camera.transform();
    }
}

/// `Render` system: turn the extracted [`PreparedEditorFrame`] into GPU-ready
/// form. Lazily creates the egui GPU resources (from the shared
/// [`RenderDevice`] and the first window surface's format), applies texture
/// deltas with deferred frees, (re)binds the viewport's view target as an
/// egui texture, tessellates, and uploads the frame slot's buffers. Produces
/// the [`EguiPreparedFrame`] resource [`egui_pass`] consumes.
///
/// When the device, a window surface, or an acquired frame is unavailable yet
/// the frame is left in place — extraction merges the next frame into it, so
/// no texture delta is lost. (The acquired-frame requirement is also what
/// makes the slot's buffer writes fence-safe.)
fn prepare_egui_frame(world: &mut World) {
    if !world.contains_resource::<PreparedEditorFrame>() {
        return;
    }
    let Some(render_device) = world
        .get_resource::<RenderDevice>()
        .map(|device| (*device).clone())
    else {
        return;
    };
    let device = render_device.device().clone();

    let surface_info = {
        let Some(mut surfaces) = world.get_resource_mut::<WindowSurfaces>() else {
            return;
        };
        surfaces.values_mut().next().and_then(|surface| {
            // Only prepare against an acquired frame: its slot's fence was
            // waited in `acquire_window_frames`, so writing the slot's buffers
            // and deferred-free ring cannot race the GPU.
            if !surface.frame_in_progress() {
                return None;
            }
            surface
                .format()
                .ok()
                .map(|(format, srgb)| (format, srgb, surface.frame_index(), surface.extent()))
        })
    };
    let Some((color_format, srgb_framebuffer, frame_slot, _extent)) = surface_info else {
        return;
    };

    if !world.contains_resource::<egui_vk::EguiPipeline>() {
        match egui_vk::EguiPipeline::new(
            &device,
            color_format,
            srgb_framebuffer,
            egui_vk::EguiOptions::default(),
        ) {
            Ok(pipeline) => world.insert_resource(pipeline),
            Err(e) => {
                error!("failed to create egui pipeline: {e}");
                return;
            }
        }
    }
    if !world.contains_resource::<egui_vk::EguiTextures>() {
        match egui_vk::EguiTextures::new(&render_device) {
            Ok(textures) => world.insert_resource(textures),
            Err(e) => {
                error!("failed to create egui textures: {e}");
                return;
            }
        }
    }
    if !world.contains_resource::<egui_vk::EguiFrameResources>() {
        match egui_vk::EguiFrameResources::new(&device, MAX_FRAMES_IN_FLIGHT) {
            Ok(frames) => world.insert_resource(frames),
            Err(e) => {
                error!("failed to create egui frame resources: {e}");
                return;
            }
        }
    }

    let Some(mut frame) = world.remove_resource::<PreparedEditorFrame>() else {
        return;
    };

    {
        let mut pipeline = world
            .get_resource_mut::<egui_vk::EguiPipeline>()
            .expect("EguiPipeline was just ensured");
        let mut textures = world
            .get_resource_mut::<egui_vk::EguiTextures>()
            .expect("EguiTextures was just ensured");
        // Draining marks deltas as applied (epaint 0.36 asserts a dropped
        // TexturesDelta is empty); several deltas per texture can arrive in
        // one frame. Uploads record into the shared frame uploader, which
        // the window frame loop flushes at submit.
        for (id, deltas) in frame.textures_delta.set.drain() {
            for delta in deltas {
                if let Err(e) = textures.update_texture(&device, &mut pipeline, id, &delta) {
                    error!("failed to update egui texture {id:?}: {e}");
                }
            }
        }
        // Freed textures retire through the device's retirement ring.
        let freed: Vec<egui::TextureId> = frame.textures_delta.free.drain().collect();
        textures.free_textures(&freed);
    }

    // Bind the viewport's offscreen target as an egui texture. A resize
    // allocates new heap slots for the target, so the registration
    // refreshes whenever the handles change.
    let viewport_handles = world.get_resource::<ViewTargets>().and_then(|targets| {
        targets
            .get(RenderTarget::Viewport)
            .map(|target| (target.texture_handle(), target.sampler_handle()))
    });
    if let Some((texture, sampler)) = viewport_handles {
        let (id, handles) = {
            let viewport = world
                .get_resource::<ViewportTexture>()
                .expect("ViewportTexture registered in render world");
            (viewport.id, viewport.handles)
        };
        let needs_register = id.is_none() || handles != Some((texture, sampler));
        if needs_register {
            let id = {
                let mut textures = world
                    .get_resource_mut::<egui_vk::EguiTextures>()
                    .expect("EguiTextures was just ensured");
                if let Some(old) = id {
                    textures.free_texture(&old);
                }
                textures.register_native_texture(texture, sampler)
            };
            let mut viewport = world
                .get_resource_mut::<ViewportTexture>()
                .expect("ViewportTexture registered in render world");
            viewport.id = Some(id);
            viewport.handles = Some((texture, sampler));
        }
    }

    let pixels_per_point = frame.pixels_per_point;
    let primitives = frame
        .egui_ctx
        .tessellate(std::mem::take(&mut frame.shapes), pixels_per_point);
    {
        let mut frames = world
            .get_resource_mut::<egui_vk::EguiFrameResources>()
            .expect("EguiFrameResources was just ensured");
        if let Err(e) = frames.update(&device, frame_slot, &primitives) {
            error!("failed to upload egui frame data: {e}");
            return;
        }
    }
    world.insert_resource(EguiPreparedFrame {
        primitives,
        pixels_per_point,
        frame_slot,
    });
}

/// `Render` system: record the egui pass into the acquired swapchain image of
/// every window with a frame in progress, after the scene pass. Consumes the
/// [`EguiPreparedFrame`] resource.
fn egui_pass(world: &mut World) {
    let Some(prepared) = world.remove_resource::<EguiPreparedFrame>() else {
        return;
    };
    let (Some(pipeline), Some(textures), Some(frames)) = (
        world.get_resource::<egui_vk::EguiPipeline>(),
        world.get_resource::<egui_vk::EguiTextures>(),
        world.get_resource::<egui_vk::EguiFrameResources>(),
    ) else {
        return;
    };
    let Some(mut surfaces) = world.get_resource_mut::<WindowSurfaces>() else {
        return;
    };
    for surface in surfaces.values_mut() {
        if !surface.frame_in_progress() {
            continue;
        }
        let extent = surface.extent();
        let (Some(image_view), Some(command_buffer)) = (
            surface.current_image_view(),
            surface.current_command_buffer(),
        ) else {
            continue;
        };
        let color_attachment = RenderAttachment {
            view: image_view,
            layout: AttachmentLayout::Present,
            load: LoadOp::Clear,
            store: StoreOp::Store,
            clear: ClearValue::Color([0.0, 0.0, 0.0, 1.0]),
        };
        command_buffer.begin_rendering(&RenderPassDesc {
            render_area: Rect2d::full(extent.width, extent.height),
            layer_count: 1,
            color_attachments: std::slice::from_ref(&color_attachment),
            depth_attachment: None,
        });
        egui_vk::record_egui(
            command_buffer,
            &pipeline,
            &textures,
            &frames,
            prepared.frame_slot,
            (extent.width, extent.height),
            prepared.pixels_per_point,
            &prepared.primitives,
        );
        command_buffer.end_rendering();
    }
}

/// `Render` system (after window submit): publish the frame's feedback to the
/// main world and honor the `MOONFIELD_EDITOR_DUMP_VIEWPORT=N` debug seam.
fn editor_frame_done(world: &mut World) {
    let presented_frames = world
        .get_resource_mut::<WindowSurfaces>()
        .and_then(|mut surfaces| {
            surfaces
                .values_mut()
                .next()
                .map(|surface| surface.presented_frames())
        })
        .unwrap_or(0);
    let viewport_texture = world
        .get_resource::<ViewportTexture>()
        .and_then(|viewport| viewport.id);
    let channel = world
        .get_resource::<EditorFeedbackChannel>()
        .map(|channel| (*channel).clone())
        .expect("EditorFeedbackChannel registered in render world");
    channel.publish(EditorFeedback {
        viewport_texture,
        frames_rendered: presented_frames,
    });

    if presented_frames > 0
        && std::env::var("MOONFIELD_EDITOR_DUMP_VIEWPORT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            == Some(presented_frames)
        && let Err(error) = dump_viewport_target(world)
    {
        error!("failed to dump viewport target: {error}");
    }
}

/// Write the viewport view target's raw BGRA pixels to
/// `target/tmp/viewport_dump_<w>x<h>.raw`. Debug seam for the
/// `MOONFIELD_EDITOR_DUMP_VIEWPORT` env var.
fn dump_viewport_target(world: &World) -> Result<(), String> {
    let render_device = world
        .get_resource::<RenderDevice>()
        .ok_or_else(|| "RenderDevice missing".to_string())?;
    let targets = world
        .get_resource::<ViewTargets>()
        .ok_or_else(|| "ViewTargets missing".to_string())?;
    let target = targets
        .get(RenderTarget::Viewport)
        .ok_or_else(|| "no viewport view target".to_string())?;
    let (width, height) = target.extent();
    let pixels = target
        .read_pixels(render_device.device())
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all("target/tmp").map_err(|e| e.to_string())?;
    std::fs::write(
        format!("target/tmp/viewport_dump_{width}x{height}.raw"),
        pixels,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        EditorFeedback, EditorFeedbackChannel, EditorPlugin, PendingEditorFrame,
        PreparedEditorFrame, merge_prepared_frames,
    };
    use moonfield_app::App;
    use moonfield_render_core::WindowFrameDemand;

    fn prepared_frame() -> PreparedEditorFrame {
        PreparedEditorFrame {
            egui_ctx: egui::Context::default(),
            shapes: Vec::new(),
            textures_delta: egui::TexturesDelta::default(),
            pixels_per_point: 1.0,
            viewport_panel_points: None,
        }
    }

    #[test]
    fn test_merge_prepared_frames_preserves_texture_deltas() {
        let mut stale = prepared_frame();
        stale
            .textures_delta
            .free
            .insert(egui::TextureId::Managed(7));
        let mut latest = prepared_frame();
        let delta = egui::epaint::ImageDelta::full(
            egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
            egui::TextureOptions::default(),
        );
        latest
            .textures_delta
            .set
            .insert(egui::TextureId::Managed(1), [delta].into());

        let mut merged = merge_prepared_frames(stale, latest);

        assert!(
            merged
                .textures_delta
                .set
                .contains_key(&egui::TextureId::Managed(1))
        );
        assert!(
            merged
                .textures_delta
                .free
                .contains(&egui::TextureId::Managed(7))
        );
        // Dropping must not trip epaint's applied-deltas assert — the
        // merged delta was deliberately left applied here.
        merged.textures_delta.clear();
    }

    #[test]
    fn test_editor_feedback_channel_take_consumes_once() {
        let channel = EditorFeedbackChannel::default();

        assert!(channel.take().is_none());
        channel.publish(EditorFeedback {
            viewport_texture: Some(egui::TextureId::User(0)),
            frames_rendered: 3,
        });
        let feedback = channel.take().expect("feedback published");
        assert_eq!(feedback.frames_rendered, 3);
        assert!(channel.take().is_none());
    }

    /// Without a window (no `WinitWindow`) or a render device, extraction and
    /// the render systems must all no-op.
    #[test]
    fn test_editor_render_without_window_or_device_is_a_noop() {
        let mut app = App::new();
        app.add_plugin(EditorPlugin);

        app.render();
        app.render();

        assert_eq!(
            app.render_world()
                .get_resource::<WindowFrameDemand>()
                .map(|demand| demand.0),
            Some(false)
        );
    }

    /// A pending frame is extracted into the render world (setting frame
    /// demand), and a second publish merges into an unconsumed one.
    #[test]
    fn test_extract_editor_frame_moves_and_merges_pending_frames() {
        let mut app = App::new();
        app.add_plugin(EditorPlugin);

        app.world_mut()
            .get_resource_mut::<PendingEditorFrame>()
            .expect("PendingEditorFrame registered in build")
            .0 = Some(prepared_frame());
        app.render();
        assert!(
            app.render_world()
                .contains_resource::<PreparedEditorFrame>()
        );
        assert!(
            app.world()
                .get_resource::<PendingEditorFrame>()
                .expect("PendingEditorFrame registered in build")
                .0
                .is_none()
        );

        // The render-world frame was never consumed (no device), so the next
        // frame must merge into it rather than replace it.
        let mut second = prepared_frame();
        second
            .textures_delta
            .free
            .insert(egui::TextureId::Managed(3));
        app.world_mut()
            .get_resource_mut::<PendingEditorFrame>()
            .expect("PendingEditorFrame registered in build")
            .0 = Some(second);
        app.render();
        let merged = app
            .render_world()
            .get_resource::<PreparedEditorFrame>()
            .expect("merged frame in render world");
        assert!(
            merged
                .textures_delta
                .free
                .contains(&egui::TextureId::Managed(3))
        );
    }
}

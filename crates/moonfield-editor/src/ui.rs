//! Editor dock layout, UE5-style: viewport center, outliner (hierarchy) and
//! details (inspector) stacked in the right column, content browser below.
//!
//! The hierarchy and inspector panels read and edit the ECS world directly
//! (the single-threaded render seam). [`collect_hierarchy`] is factored out
//! as a pure world-reading function so it can be tested without a display.

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use moonfield_camera::{view_matrix, Camera, PrimaryCamera};
use moonfield_ecs::{ChildOf, Children, Entity, Name, RelationshipTarget, World};
use moonfield_math::{GlobalTransform, Transform};
use moonfield_scene::SceneRegistry;

use crate::interaction::{
    hit_test, screen_to_ray, world_to_screen, world_trs_to_local, GizmoDrag, GizmoFrame,
    GizmoHandle, GizmoMode, OrbitCamera,
};

/// Editor panel tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Hierarchy,
    Inspector,
    Viewport,
    Content,
}

/// Per-frame context handed to the tab viewer.
pub struct TabContext<'w> {
    /// The ECS world: panels read it (hierarchy) and edit it (inspector).
    pub world: &'w mut World,
    /// The entity the inspector edits; set by hierarchy clicks.
    pub selection: &'w mut Option<Entity>,
    /// Content panel's asset-load field state.
    pub load_state: &'w mut LoadAssetState,
    /// Content panel's scene Save/Load field state.
    pub scene_state: &'w mut SceneIoState,
    /// The viewport scene texture, once registered with the egui texture
    /// store (`egui_vk::EguiTextures`).
    pub viewport_texture: Option<egui::TextureId>,
    /// The viewport panel size in points, reported back to the runner so it
    /// can resize the offscreen target.
    pub viewport_size_points: Option<egui::Vec2>,
    /// The editor-owned orbit camera; initialized from the primary camera's
    /// pose on the first viewport frame, written back by the runner.
    pub camera: &'w mut Option<OrbitCamera>,
    /// The active gizmo operation, switched with W/E/R in the viewport.
    pub gizmo_mode: &'w mut GizmoMode,
    /// The in-progress gizmo drag, if any.
    pub gizmo_drag: &'w mut Option<GizmoDrag>,
}

/// Content panel state for loading assets: a path field and the last
/// load's status message (error or success).
///
/// File dialogs need a native-dialog dependency; for now the path is typed
/// (or pasted) directly. The load itself is synchronous — large glTFs will
/// stall the frame (async loading is a known debt).
#[derive(Default)]
pub struct LoadAssetState {
    /// The asset path to load (`.gltf`/`.glb`; splat or mesh by content).
    pub path: String,
    /// Status of the last load attempt.
    pub message: Option<String>,
}

/// Content panel state for scene save/load: a `.gltf` path field and the
/// last operation's status message (error or success).
///
/// Save/load go through the world's `SceneRegistry` resource via
/// moonfield-scene's file APIs, synchronously on the UI thread (same known
/// debt as the asset loader).
#[derive(Default)]
pub struct SceneIoState {
    /// The scene path to save to / load from.
    pub path: String,
    /// Status of the last save/load attempt.
    pub message: Option<String>,
}

/// Build the initial dock layout, UE5-style: the viewport dominates the
/// center, the right column holds the outliner (hierarchy) over the details
/// (inspector), and the content browser sits in a bottom strip.
pub fn initial_dock_state() -> DockState<Tab> {
    let mut state = DockState::new(vec![Tab::Viewport]);
    let surface = state.main_surface_mut();
    // Right column (~22% width): outliner over details.
    // (`split_*` returns [old, new]; `fraction` is the old node's share.)
    let [viewport, right] = surface.split_right(NodeIndex::root(), 0.78, vec![Tab::Hierarchy]);
    let [_outliner, _details] = surface.split_below(right, 0.5, vec![Tab::Inspector]);
    // Content browser below the viewport (~28% of the height).
    let [_viewport, _content] = surface.split_below(viewport, 0.72, vec![Tab::Content]);
    state
}

/// Render the dock area covering the whole window.
///
/// egui_dock 0.21 dropped `DockArea::show(ctx, …)`; the 0.18 equivalent was
/// a transparent, margin-less `CentralPanel` around `show_inside`, mirrored
/// here (`Frame::NONE` is transparent and margin-less). `show_inside`
/// derives the dock style from the egui style when none is set.
pub fn show(ui: &mut egui::Ui, dock_state: &mut DockState<Tab>, context: &mut TabContext) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            DockArea::new(dock_state).show_inside(ui, &mut EditorTabViewer { context });
        });
}

struct EditorTabViewer<'a, 'w> {
    context: &'a mut TabContext<'w>,
}

impl TabViewer for EditorTabViewer<'_, '_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Tab::Hierarchy => "Outliner".into(),
            Tab::Inspector => "Details".into(),
            Tab::Viewport => "Viewport".into(),
            Tab::Content => "Content Browser".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Tab::Hierarchy => hierarchy_panel(ui, self.context.world, self.context.selection),
            Tab::Inspector => inspector_panel(ui, self.context.world, self.context.selection),
            Tab::Viewport => viewport_panel(ui, self.context),
            Tab::Content => content_panel(
                ui,
                self.context.world,
                self.context.selection,
                self.context.load_state,
                self.context.scene_state,
            ),
        }
    }
}

// ---------------------------------------------------------------------
// Hierarchy panel
// ---------------------------------------------------------------------

/// One row of the hierarchy tree view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyEntry {
    /// The entity shown on this row.
    pub entity: Entity,
    /// Display label: the `Name` if any, otherwise the raw entity.
    pub label: String,
    /// Nesting depth (roots are 0).
    pub depth: usize,
}

/// Flatten the world's entity hierarchy into display order: every entity
/// without a [`ChildOf`] is a root (sorted for stable display), followed
/// recursively by its [`Children`].
pub fn collect_hierarchy(world: &World) -> Vec<HierarchyEntry> {
    // `Option<&ChildOf>` iterates every entity, yielding `None` for roots.
    let mut roots: Vec<Entity> = world
        .query::<Option<&ChildOf>>()
        .filter(|(_, child_of)| child_of.is_none())
        .map(|(entity, _)| entity)
        .collect();
    roots.sort_by_key(|e| e.to_bits());

    let mut entries = Vec::new();
    for root in roots {
        collect_subtree(world, root, 0, &mut entries);
    }
    entries
}

fn collect_subtree(world: &World, entity: Entity, depth: usize, entries: &mut Vec<HierarchyEntry>) {
    entries.push(HierarchyEntry {
        entity,
        label: entity_label(world, entity),
        depth,
    });
    if let Some(children) = world.get_component::<Children>(entity) {
        for &child in children.entities() {
            collect_subtree(world, child, depth + 1, entries);
        }
    }
}

fn entity_label(world: &World, entity: Entity) -> String {
    match world.get_component::<Name>(entity) {
        Some(name) => name.to_string(),
        None => format!("{entity:?}"),
    }
}

fn hierarchy_panel(ui: &mut egui::Ui, world: &mut World, selection: &mut Option<Entity>) {
    let entries = collect_hierarchy(world);
    if entries.is_empty() {
        ui.label("Scene is empty");
        return;
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        for entry in entries {
            ui.horizontal(|ui| {
                ui.add_space(entry.depth as f32 * 16.0);
                let selected = *selection == Some(entry.entity);
                if ui
                    .selectable_label(selected, &entry.label)
                    .on_hover_text(format!("{:?}", entry.entity))
                    .clicked()
                {
                    *selection = Some(entry.entity);
                }
            });
        }
    });
}

// ---------------------------------------------------------------------
// Content panel
// ---------------------------------------------------------------------

/// The content browser: asset import and scene save/load. (A file listing
/// arrives with the native file dialog; for now both rows are typed paths.)
fn content_panel(
    ui: &mut egui::Ui,
    world: &mut World,
    selection: &mut Option<Entity>,
    load_state: &mut LoadAssetState,
    scene_state: &mut SceneIoState,
) {
    // Asset loading: type/paste a glTF path, load it synchronously into the
    // world (asset store + entity with the matching handle component —
    // splat cloud or mesh, decided by file content).
    ui.horizontal(|ui| {
        ui.label("Asset:");
        ui.add(
            egui::TextEdit::singleline(&mut load_state.path)
                .hint_text("path/to/asset.gltf")
                .desired_width(f32::INFINITY),
        );
        if ui.button("Load").clicked() && !load_state.path.trim().is_empty() {
            let path = std::path::PathBuf::from(load_state.path.trim());
            load_state.message = Some(match crate::scene_io::load_asset(world, &path) {
                Ok(entity) => {
                    *selection = Some(entity);
                    format!("Loaded {}", path.display())
                }
                Err(e) => format!("Load failed: {e}"),
            });
        }
    });
    if let Some(message) = &load_state.message {
        ui.small(message);
    }

    // Scene save/load: the world's registered entities ⇄ a .gltf document
    // via moonfield-scene, using the SceneRegistry resource.
    ui.horizontal(|ui| {
        ui.label("Scene:");
        ui.add(
            egui::TextEdit::singleline(&mut scene_state.path)
                .hint_text("scene.gltf")
                .desired_width(f32::INFINITY),
        );
        let path_is_empty = scene_state.path.trim().is_empty();
        if ui.button("Save").clicked() && !path_is_empty {
            let path = std::path::PathBuf::from(scene_state.path.trim());
            scene_state.message = Some(match world.get_resource::<SceneRegistry>() {
                Some(registry) => {
                    match moonfield_scene::save_scene_to_file(world, &registry, &path) {
                        Ok(()) => format!("Saved {}", path.display()),
                        Err(e) => format!("Save failed: {e}"),
                    }
                }
                None => "Save failed: SceneRegistry resource missing".to_string(),
            });
        }
        if ui.button("Load").clicked() && !path_is_empty {
            let path = std::path::PathBuf::from(scene_state.path.trim());
            // `load_scene_from_file` needs `&mut World` while the registry
            // lives inside the world's resource storage: take the registry
            // out, use it, and put it back (also on the error path).
            scene_state.message = Some(match world.remove_resource::<SceneRegistry>() {
                Some(registry) => {
                    let result = moonfield_scene::load_scene_from_file(world, &registry, &path);
                    world.insert_resource(registry);
                    match result {
                        Ok(roots) => {
                            *selection = roots.first().copied();
                            format!("Loaded {} ({} roots)", path.display(), roots.len())
                        }
                        Err(e) => format!("Load failed: {e}"),
                    }
                }
                None => "Load failed: SceneRegistry resource missing".to_string(),
            });
        }
    });
    if let Some(message) = &scene_state.message {
        ui.small(message);
    }
}

// ---------------------------------------------------------------------
// Inspector panel
// ---------------------------------------------------------------------

fn inspector_panel(ui: &mut egui::Ui, world: &mut World, selection: &mut Option<Entity>) {
    let Some(entity) = *selection else {
        ui.label("Select an entity in the Outliner panel");
        return;
    };
    if !world.contains(entity) {
        ui.label("Selected entity no longer exists");
        *selection = None;
        return;
    }

    if let Some(name) = world.get_component::<Name>(entity) {
        ui.heading(name.to_string());
    } else {
        ui.heading(format!("{entity:?}"));
    }
    ui.separator();

    // Generic path: every component type registered in the InspectorRegistry
    // gets auto-generated Reflect-driven editing UI. (Snapshot the registry
    // first so the resource borrow is released before the world is mutated.)
    let components = world
        .get_resource::<crate::registry::InspectorRegistry>()
        .map(|registry| registry.components())
        .unwrap_or_default();
    crate::registry::show_components(ui, world, entity, &components);
}

// ---------------------------------------------------------------------
// Viewport panel
// ---------------------------------------------------------------------

/// The primary camera: the first entity with `Camera` + [`PrimaryCamera`] +
/// `GlobalTransform`. The viewport panel's interaction overlay and the
/// render-feature scene pass both key off it, so they see the same view.
pub(crate) fn primary_camera(world: &World) -> Option<(Camera, GlobalTransform)> {
    for (entity, (cam, global)) in world.query::<(&Camera, &GlobalTransform)>() {
        if world.get_component::<PrimaryCamera>(entity).is_some() {
            return Some((*cam, *global));
        }
    }
    None
}

/// The viewport: the scene texture as an interactive image. Right-drag
/// orbits the editor camera, middle-drag pans, the wheel zooms, and the
/// left button drives the transform gizmo on the selected entity.
fn viewport_panel(ui: &mut egui::Ui, context: &mut TabContext) {
    let rect = ui.available_rect_before_wrap();
    context.viewport_size_points = Some(rect.size());

    // Camera: initialize once from the primary camera's pose, then own it so
    // the pose is ready on the first textured frame.
    if context.camera.is_none() {
        let first = context.world.query::<(&Transform, &PrimaryCamera)>().next();
        if let Some((_, (transform, _))) = first {
            *context.camera = Some(OrbitCamera::from_transform(transform));
        }
    }

    let Some(texture_id) = context.viewport_texture else {
        ui.centered_and_justified(|ui| {
            ui.label("Initializing viewport…");
        });
        return;
    };

    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

    // Debug seam: MOONFIELD_EDITOR_DEBUG_GIZMO=1 logs the viewport response
    // state every frame.
    if std::env::var_os("MOONFIELD_EDITOR_DEBUG_GIZMO").is_some() {
        eprintln!(
            "[gizmo-debug] hovered={} down_on={} dragged={} drag_started={} interact_pos={:?}",
            response.hovered(),
            response.is_pointer_button_down_on(),
            response.dragged(),
            response.drag_started(),
            response.interact_pointer_pos(),
        );
    }
    ui.painter().image(
        texture_id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    if let Some(camera) = context.camera.as_mut() {
        if response.dragged_by(egui::PointerButton::Secondary) {
            camera.orbit(response.drag_delta());
        } else if response.dragged_by(egui::PointerButton::Middle) {
            camera.pan(response.drag_delta());
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                camera.zoom(scroll);
            }
        }
    }

    // W/E/R switch the gizmo mode unless a text field has keyboard focus.
    if !ui.ctx().egui_wants_keyboard_input() {
        for (key, mode) in [
            (egui::Key::W, GizmoMode::Translate),
            (egui::Key::E, GizmoMode::Rotate),
            (egui::Key::R, GizmoMode::Scale),
        ] {
            if ui.input(|i| i.key_pressed(key)) {
                *context.gizmo_mode = mode;
            }
        }
    }

    gizmo(ui, context, rect, &response);

    // Status overlay, top-left corner: the active gizmo mode (so W/E/R give
    // visible feedback), the controls, and a hint when nothing is selected.
    let mode_label = match *context.gizmo_mode {
        GizmoMode::Translate => "Translate",
        GizmoMode::Rotate => "Rotate",
        GizmoMode::Scale => "Scale",
    };
    let has_selection = (*context.selection)
        .filter(|e| context.world.contains(*e))
        .is_some();
    let mut lines = vec![
        format!("Mode: {mode_label}  (W/E/R to switch)"),
        "RMB orbit · MMB pan · wheel zoom · LMB drag gizmo".to_string(),
    ];
    if !has_selection {
        lines.push("Select an entity in the Hierarchy to show its gizmo".to_string());
    }
    for (row, line) in lines.iter().enumerate() {
        ui.painter().text(
            rect.min + egui::vec2(8.0, 8.0 + row as f32 * 16.0),
            egui::Align2::LEFT_TOP,
            line,
            egui::FontId::proportional(12.0),
            egui::Color32::from_white_alpha(160),
        );
    }
}

/// Gizmo hit-testing, dragging, and drawing for the selected entity.
fn gizmo(ui: &egui::Ui, context: &mut TabContext, rect: egui::Rect, response: &egui::Response) {
    let mode = *context.gizmo_mode;
    let aspect = rect.width() / rect.height().max(1.0);
    let Some((camera, camera_global)) = primary_camera(context.world) else {
        return;
    };
    let view_proj = camera.projection_matrix(aspect) * view_matrix(&camera_global);

    let Some(entity) = (*context.selection)
        .filter(|e| context.world.contains(*e))
        .filter(|e| context.world.get_component::<Transform>(*e).is_some())
    else {
        *context.gizmo_drag = None;
        return;
    };
    // glam's decompose returns (scale, rotation, translation); the gizmo
    // pipeline works in (translation, rotation, scale) order — reorder
    // explicitly at this boundary.
    let Some((translation, rotation, scale)) = context
        .world
        .get_component::<GlobalTransform>(entity)
        .map(|g| {
            let (scale, rotation, translation) = g.affine().to_scale_rotation_translation();
            (translation, rotation, scale)
        })
    else {
        *context.gizmo_drag = None;
        return;
    };

    let frame = GizmoFrame::new(translation, rotation);
    let pointer = response.hover_pos().filter(|_| response.hovered());
    let center = world_to_screen(frame.origin, view_proj, rect);
    let mut drag = *context.gizmo_drag;

    // An active drag applies to the world until the button is released.
    if let Some(active) = drag {
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (Some(pointer), Some(center)) =
                (pointer.or(response.interact_pointer_pos()), center)
            {
                let ray = screen_to_ray(pointer, rect, view_proj);
                if let Some(trs) = active.apply(ray, pointer, center) {
                    let parent =
                        context
                            .world
                            .get_component::<ChildOf>(entity)
                            .and_then(|child_of| {
                                context
                                    .world
                                    .get_component::<GlobalTransform>(child_of.parent())
                                    .map(|g| g.affine())
                            });
                    let local = world_trs_to_local(trs, parent);
                    if let Some(mut transform) =
                        context.world.get_component_mut::<Transform>(entity)
                    {
                        *transform = local;
                    }
                }
            }
        } else {
            drag = None;
        }
    }

    // A new drag starts when the primary button goes down on a handle.
    if drag.is_none() && response.drag_started_by(egui::PointerButton::Primary) {
        if let (Some(pointer), Some(center)) = (response.interact_pointer_pos(), center) {
            // Debug seam: MOONFIELD_EDITOR_DEBUG_GIZMO=1 logs drag starts.
            if std::env::var_os("MOONFIELD_EDITOR_DEBUG_GIZMO").is_some() {
                let hit = hit_test(mode, &frame, view_proj, rect, pointer);
                eprintln!("[gizmo-debug] drag_started pointer={pointer:?} hit={hit:?}");
            }
            if let Some(handle) = hit_test(mode, &frame, view_proj, rect, pointer) {
                drag = GizmoDrag::begin(
                    mode,
                    handle,
                    &frame,
                    screen_to_ray(pointer, rect, view_proj),
                    pointer,
                    center,
                    (translation, rotation, scale),
                );
            }
        }
    }
    *context.gizmo_drag = drag;

    // Hover highlight only while idle; while dragging the dragged handle
    // stays highlighted.
    let highlighted = match drag {
        Some(active) => Some(active.handle()),
        None => pointer.and_then(|p| hit_test(mode, &frame, view_proj, rect, p)),
    };
    draw_gizmo(ui.painter(), mode, &frame, view_proj, rect, highlighted);
}

/// Draw the gizmo handles over the viewport image. X/Y/Z are red/green/blue;
/// the hovered or dragged handle turns yellow.
fn draw_gizmo(
    painter: &egui::Painter,
    mode: GizmoMode,
    frame: &GizmoFrame,
    view_proj: moonfield_math::Mat4,
    rect: egui::Rect,
    highlighted: Option<GizmoHandle>,
) {
    const AXIS_COLORS: [egui::Color32; 3] = [
        egui::Color32::from_rgb(235, 64, 52),
        egui::Color32::from_rgb(104, 213, 78),
        egui::Color32::from_rgb(60, 130, 246),
    ];
    let color = |axis: usize| {
        if highlighted == Some(GizmoHandle::Axis(axis)) {
            egui::Color32::YELLOW
        } else {
            AXIS_COLORS[axis]
        }
    };
    let stroke = |axis: usize| egui::Stroke::new(3.0, color(axis));

    match mode {
        GizmoMode::Translate => {
            for axis in 0..3 {
                if let Some((start, end)) = frame.axis_segment(axis, view_proj, rect) {
                    painter.line_segment([start, end], stroke(axis));
                    // Arrowhead: two barbs angled back from the tip.
                    let dir = (end - start).normalized();
                    for angle in [2.6, -2.6] {
                        let barb = egui::emath::Rot2::from_angle(angle) * dir * 10.0;
                        painter.line_segment([end, end + barb], stroke(axis));
                    }
                }
            }
        }
        GizmoMode::Scale => {
            for axis in 0..3 {
                if let Some((start, end)) = frame.axis_segment(axis, view_proj, rect) {
                    painter.line_segment([start, end], stroke(axis));
                    painter.rect_filled(
                        egui::Rect::from_center_size(end, egui::vec2(8.0, 8.0)),
                        0.0,
                        color(axis),
                    );
                }
            }
            if let Some(center) = world_to_screen(frame.origin, view_proj, rect) {
                let uniform_color = if highlighted == Some(GizmoHandle::Uniform) {
                    egui::Color32::YELLOW
                } else {
                    egui::Color32::from_white_alpha(200)
                };
                painter.circle(
                    center,
                    6.0,
                    uniform_color,
                    egui::Stroke::new(1.5, uniform_color),
                );
            }
        }
        GizmoMode::Rotate => {
            for axis in 0..3 {
                let points = frame.ring_points(axis, view_proj, rect);
                if points.len() >= 2 {
                    painter.add(egui::Shape::line(points, stroke(axis)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_math::Vec3;

    fn named(world: &mut World, name: &str) -> Entity {
        world.spawn((Name::new(name),))
    }

    #[test]
    fn test_collect_hierarchy_empty_world() {
        let world = World::new();
        assert!(collect_hierarchy(&world).is_empty());
    }

    #[test]
    fn test_collect_hierarchy_roots_and_children_in_order() {
        let mut world = World::new();
        world.register_hierarchy();

        let parent = named(&mut world, "Parent");
        let child = named(&mut world, "Child");
        world.insert_component(child, ChildOf(parent));
        let grandchild = named(&mut world, "Grandchild");
        world.insert_component(grandchild, ChildOf(child));
        named(&mut world, "Other Root");

        let entries = collect_hierarchy(&world);
        let find = |label: &str| entries.iter().find(|e| e.label == label).unwrap();

        assert_eq!(find("Parent").depth, 0);
        assert_eq!(find("Child").depth, 1);
        assert_eq!(find("Grandchild").depth, 2);
        assert_eq!(find("Other Root").depth, 0);

        // Parents appear before their children in display order.
        let pos = |label: &str| entries.iter().position(|e| e.label == label).unwrap();
        assert!(pos("Parent") < pos("Child"));
        assert!(pos("Child") < pos("Grandchild"));
        assert_eq!(entries.len(), 4);
    }

    #[test]
    fn test_collect_hierarchy_unnamed_entity_falls_back_to_debug() {
        let mut world = World::new();
        let e = world.spawn(());
        let entries = collect_hierarchy(&world);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, format!("{e:?}"));
    }

    /// The viewport panel runs headless: without a registered texture it
    /// shows its initialization state and still reports its size.
    #[test]
    fn test_viewport_panel_without_texture_reports_size() {
        let mut world = World::new();
        let mut selection = None;
        let mut load_state = LoadAssetState::default();
        let mut scene_state = SceneIoState::default();
        let mut camera = None;
        let mut gizmo_mode = GizmoMode::Translate;
        let mut gizmo_drag = None;

        let ctx = egui::Context::default();
        let mut reported = None;
        // The output's texture deltas are irrelevant here; epaint 0.36
        // debug-asserts that a dropped `TexturesDelta` was fully applied.
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut context = TabContext {
                world: &mut world,
                selection: &mut selection,
                load_state: &mut load_state,
                scene_state: &mut scene_state,
                viewport_texture: None,
                viewport_size_points: None,
                camera: &mut camera,
                gizmo_mode: &mut gizmo_mode,
                gizmo_drag: &mut gizmo_drag,
            };
            viewport_panel(ui, &mut context);
            reported = context.viewport_size_points;
        })
        .drop_without_applying_deltas();
        assert!(reported.is_some());
        assert!(camera.is_none()); // no primary camera in the world
    }

    /// With a primary camera in the world, the viewport panel initializes
    /// the editor orbit camera from the camera entity's pose.
    #[test]
    fn test_viewport_panel_initializes_orbit_camera() {
        let mut world = World::new();
        world.spawn((
            Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
            PrimaryCamera,
        ));
        let mut selection = None;
        let mut load_state = LoadAssetState::default();
        let mut scene_state = SceneIoState::default();
        let mut camera = None;
        let mut gizmo_mode = GizmoMode::Translate;
        let mut gizmo_drag = None;

        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut context = TabContext {
                world: &mut world,
                selection: &mut selection,
                load_state: &mut load_state,
                scene_state: &mut scene_state,
                viewport_texture: None,
                viewport_size_points: None,
                camera: &mut camera,
                gizmo_mode: &mut gizmo_mode,
                gizmo_drag: &mut gizmo_drag,
            };
            viewport_panel(ui, &mut context);
        })
        .drop_without_applying_deltas();

        let camera = camera.expect("orbit camera initialized");
        assert!(camera.distance > 0.0);
        // The rebuilt pose matches the original camera position.
        let rebuilt = camera.transform();
        assert!(rebuilt.translation.distance(Vec3::new(0.0, 2.5, 6.0)) < 1e-4);
    }

    /// A selected entity with a world transform gets its gizmo drawn: with a
    /// (fake) texture id the panel must emit the translate gizmo's line
    /// shapes on top of the image.
    #[test]
    fn test_viewport_panel_draws_gizmo_for_selection() {
        let mut world = World::new();
        world.spawn((
            moonfield_camera::Camera::default(),
            PrimaryCamera,
            Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
            GlobalTransform::from(
                Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
            ),
        ));
        let cube = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY));

        let mut selection = Some(cube);
        let mut load_state = LoadAssetState::default();
        let mut scene_state = SceneIoState::default();
        let mut camera = None;
        let mut gizmo_mode = GizmoMode::Translate;
        let mut gizmo_drag = None;

        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            let mut context = TabContext {
                world: &mut world,
                selection: &mut selection,
                load_state: &mut load_state,
                scene_state: &mut scene_state,
                viewport_texture: Some(egui::TextureId::User(1)),
                viewport_size_points: None,
                camera: &mut camera,
                gizmo_mode: &mut gizmo_mode,
                gizmo_drag: &mut gizmo_drag,
            };
            viewport_panel(ui, &mut context);
        });
        let line_count = output
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::LineSegment { .. }))
            .count();
        output.drop_without_applying_deltas();
        // 3 axis arrows, 2 barbs each.
        assert_eq!(line_count, 9, "translate gizmo lines drawn");
    }

    /// W/E/R switch the gizmo mode when no text field has keyboard focus.
    #[test]
    fn test_viewport_panel_mode_keys() {
        let mut world = World::new();
        let mut selection = None;
        let mut load_state = LoadAssetState::default();
        let mut scene_state = SceneIoState::default();
        let mut camera = None;
        let mut gizmo_mode = GizmoMode::Translate;
        let mut gizmo_drag = None;

        let ctx = egui::Context::default();
        let press = |key| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1280.0, 720.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
            ..Default::default()
        };
        let mut run = |input: egui::RawInput,
                       world: &mut World,
                       selection: &mut Option<Entity>,
                       gizmo_mode: &mut GizmoMode| {
            ctx.run_ui(input, |ui| {
                let mut context = TabContext {
                    world,
                    selection,
                    load_state: &mut load_state,
                    scene_state: &mut scene_state,
                    viewport_texture: Some(egui::TextureId::User(1)),
                    viewport_size_points: None,
                    camera: &mut camera,
                    gizmo_mode,
                    gizmo_drag: &mut gizmo_drag,
                };
                viewport_panel(ui, &mut context);
            })
            .drop_without_applying_deltas();
        };

        run(
            press(egui::Key::E),
            &mut world,
            &mut selection,
            &mut gizmo_mode,
        );
        assert_eq!(gizmo_mode, GizmoMode::Rotate);
        run(
            press(egui::Key::R),
            &mut world,
            &mut selection,
            &mut gizmo_mode,
        );
        assert_eq!(gizmo_mode, GizmoMode::Scale);
        run(
            press(egui::Key::W),
            &mut world,
            &mut selection,
            &mut gizmo_mode,
        );
        assert_eq!(gizmo_mode, GizmoMode::Translate);
    }

    /// Dragging a translate handle with synthetic pointer events keeps the
    /// entity's transform finite and the gizmo visible throughout the drag.
    #[test]
    fn test_viewport_panel_translate_drag_stays_alive() {
        let mut world = World::new();
        let camera_transform = Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y);
        world.spawn((
            moonfield_camera::Camera::default(),
            PrimaryCamera,
            camera_transform,
            GlobalTransform::from(camera_transform),
        ));
        let cube = world.spawn((Transform::IDENTITY, GlobalTransform::IDENTITY));

        let mut selection = Some(cube);
        let mut load_state = LoadAssetState::default();
        let mut scene_state = SceneIoState::default();
        let mut camera = None;
        let mut gizmo_mode = GizmoMode::Translate;
        let mut gizmo_drag: Option<GizmoDrag> = None;

        let ctx = egui::Context::default();
        let mut frame = 0u64;
        // Run one UI frame; return the line segments drawn this frame.
        let mut run = |events: Vec<egui::Event>,
                       world: &mut World,
                       selection: &mut Option<Entity>,
                       gizmo_mode: &mut GizmoMode,
                       gizmo_drag: &mut Option<GizmoDrag>| {
            frame += 1;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(1280.0, 720.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut context = TabContext {
                        world,
                        selection,
                        load_state: &mut load_state,
                        scene_state: &mut scene_state,
                        viewport_texture: Some(egui::TextureId::User(1)),
                        viewport_size_points: None,
                        camera: &mut camera,
                        gizmo_mode,
                        gizmo_drag,
                    };
                    viewport_panel(ui, &mut context);
                },
            );
            let lines: Vec<([egui::Pos2; 2], egui::Color32)> = output
                .shapes
                .iter()
                .filter_map(|s| match s.shape {
                    egui::Shape::LineSegment { points, stroke } => Some((points, stroke.color)),
                    _ => None,
                })
                .collect();
            output.drop_without_applying_deltas();
            lines
        };

        // Frame 1: idle — the gizmo is drawn (3 axes + 6 barbs). Locate the
        // X-axis handle from the *drawn* shapes (the longest red line), so
        // the test uses the panel's real coordinates.
        let lines = run(
            vec![],
            &mut world,
            &mut selection,
            &mut gizmo_mode,
            &mut gizmo_drag,
        );
        assert_eq!(lines.len(), 9);
        let red = egui::Color32::from_rgb(235, 64, 52);
        let [start, end] = lines
            .iter()
            .filter(|(_, color)| *color == red)
            .map(|(points, _)| *points)
            .max_by(|a, b| (a[1] - a[0]).length().total_cmp(&(b[1] - b[0]).length()))
            .expect("X axis line drawn");
        let grab = start + (end - start) * 0.5;

        // Press on the handle.
        run(
            vec![
                egui::Event::PointerMoved(grab),
                egui::Event::PointerButton {
                    pos: grab,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            &mut world,
            &mut selection,
            &mut gizmo_mode,
            &mut gizmo_drag,
        );
        // Drag a few pixels along the handle, one move per frame; the gizmo
        // must stay drawn every frame. egui postpones the click/drag
        // decision until the pointer has moved past max_click_dist (6 pt)
        // from the press point, so the drag begins mid-sequence.
        for step in 1..=5 {
            let pos = grab + (end - start) * 0.05 * step as f32;
            let lines = run(
                vec![egui::Event::PointerMoved(pos)],
                &mut world,
                &mut selection,
                &mut gizmo_mode,
                &mut gizmo_drag,
            );
            assert_eq!(lines.len(), 9, "gizmo vanished at drag step {step}");
            let t = world.get_component::<Transform>(cube).unwrap();
            assert!(
                t.translation.is_finite(),
                "translation went non-finite at drag step {step}: {:?}",
                t.translation
            );
        }
        assert!(gizmo_drag.is_some(), "drag never begun");
        // The drag actually moved the entity along +X; rotation and scale
        // are untouched (regression: a tuple-order mixup once wrote the
        // translation into the scale field and NaN'd the rotation).
        let t = world.get_component::<Transform>(cube).unwrap();
        assert!(t.translation.x > 0.01, "translation={:?}", t.translation);
        assert_eq!(t.scale, Vec3::ONE);
        assert!((t.rotation.length() - 1.0).abs() < 1e-4);

        // Release: drag state clears, gizmo still drawn.
        run(
            vec![egui::Event::PointerButton {
                pos: grab,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
            &mut world,
            &mut selection,
            &mut gizmo_mode,
            &mut gizmo_drag,
        );
        assert!(gizmo_drag.is_none());
    }
}

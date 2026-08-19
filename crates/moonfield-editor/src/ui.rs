//! Editor dock layout: hierarchy, inspector and viewport panels.
//!
//! The hierarchy and inspector panels read and edit the ECS world directly
//! (the single-threaded render seam). [`collect_hierarchy`] is factored out
//! as a pure world-reading function so it can be tested without a display.

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use moonfield_ecs::{ChildOf, Children, Entity, Name, RelationshipTarget, World};

/// Editor panel tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Hierarchy,
    Inspector,
    Viewport,
}

/// Per-frame context handed to the tab viewer.
pub struct TabContext<'w> {
    /// The ECS world: panels read it (hierarchy) and edit it (inspector).
    pub world: &'w mut World,
    /// The entity the inspector edits; set by hierarchy clicks.
    pub selection: &'w mut Option<Entity>,
    /// Hierarchy panel's PLY-load field state.
    pub load_state: &'w mut LoadSplatState,
    /// The viewport scene texture, once registered with the egui renderer.
    pub viewport_texture: Option<egui::TextureId>,
    /// The viewport panel size in points, reported back to the runner so it
    /// can resize the offscreen target.
    pub viewport_size_points: Option<egui::Vec2>,
}

/// Hierarchy panel state for loading splat clouds: a path field and the
/// last load's status message (error or success).
///
/// File dialogs need a native-dialog dependency; for now the path is typed
/// (or pasted) directly. The load itself is synchronous — large PLYs will
/// stall the frame (async loading is a known debt).
#[derive(Default)]
pub struct LoadSplatState {
    /// The PLY path to load.
    pub path: String,
    /// Status of the last load attempt.
    pub message: Option<String>,
}

/// Build the initial dock layout: hierarchy on the left, inspector on the
/// right, viewport in the center.
pub fn initial_dock_state() -> DockState<Tab> {
    let mut state = DockState::new(vec![Tab::Viewport]);
    let surface = state.main_surface_mut();
    let [_hierarchy, rest] = surface.split_left(NodeIndex::root(), 0.22, vec![Tab::Hierarchy]);
    let [_inspector, _viewport] = surface.split_right(rest, 0.75, vec![Tab::Inspector]);
    state
}

/// Render the dock area covering the whole window.
pub fn show(ctx: &egui::Context, dock_state: &mut DockState<Tab>, context: &mut TabContext) {
    DockArea::new(dock_state)
        .style(egui_dock::Style::from_egui(ctx.style().as_ref()))
        .show(ctx, &mut EditorTabViewer { context });
}

struct EditorTabViewer<'a, 'w> {
    context: &'a mut TabContext<'w>,
}

impl TabViewer for EditorTabViewer<'_, '_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        match tab {
            Tab::Hierarchy => "Hierarchy".into(),
            Tab::Inspector => "Inspector".into(),
            Tab::Viewport => "Viewport".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Tab::Hierarchy => hierarchy_panel(
                ui,
                self.context.world,
                self.context.selection,
                self.context.load_state,
            ),
            Tab::Inspector => inspector_panel(ui, self.context.world, self.context.selection),
            Tab::Viewport => viewport_panel(ui, self.context),
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

fn hierarchy_panel(
    ui: &mut egui::Ui,
    world: &mut World,
    selection: &mut Option<Entity>,
    load_state: &mut LoadSplatState,
) {
    // Splat loading: type/paste a PLY path, load it synchronously into the
    // world (asset store + entity with SplatCloudHandle).
    ui.horizontal(|ui| {
        ui.label("PLY:");
        ui.add(
            egui::TextEdit::singleline(&mut load_state.path)
                .hint_text("path/to/cloud.ply")
                .desired_width(f32::INFINITY),
        );
        if ui.button("Load").clicked() && !load_state.path.trim().is_empty() {
            let path = std::path::PathBuf::from(load_state.path.trim());
            load_state.message = Some(match crate::scene_io::load_splat_cloud(world, &path) {
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
    ui.separator();

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
// Inspector panel
// ---------------------------------------------------------------------

fn inspector_panel(ui: &mut egui::Ui, world: &mut World, selection: &mut Option<Entity>) {
    let Some(entity) = *selection else {
        ui.label("Select an entity in the Hierarchy panel");
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

fn viewport_panel(ui: &mut egui::Ui, context: &mut TabContext) {
    let rect = ui.available_rect_before_wrap();
    context.viewport_size_points = Some(rect.size());

    match context.viewport_texture {
        Some(texture_id) => {
            let image = egui::Image::new(egui::load::SizedTexture::new(texture_id, rect.size()))
                .fit_to_exact_size(rect.size());
            ui.put(rect, image);
        }
        None => {
            ui.centered_and_justified(|ui| {
                ui.label("Initializing viewport…");
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

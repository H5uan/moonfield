//! Editor dock layout: hierarchy, inspector and viewport panels.

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};

/// Editor panel tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Hierarchy,
    Inspector,
    Viewport,
}

/// Per-frame context handed to the tab viewer.
pub struct TabContext {
    /// The viewport scene texture, once registered with the egui renderer.
    pub viewport_texture: Option<egui::TextureId>,
    /// The viewport panel size in points, reported back to the runner so it
    /// can resize the offscreen target.
    pub viewport_size_points: Option<egui::Vec2>,
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

struct EditorTabViewer<'a> {
    context: &'a mut TabContext,
}

impl TabViewer for EditorTabViewer<'_> {
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
            Tab::Hierarchy => hierarchy_panel(ui),
            Tab::Inspector => inspector_panel(ui),
            Tab::Viewport => viewport_panel(ui, self.context),
        }
    }
}

fn hierarchy_panel(ui: &mut egui::Ui) {
    // Placeholder until the ECS component registry lands.
    ui.label("Scene");
    for entity in ["Main Camera", "Directional Light", "Triangle"] {
        ui.indent(entity, |ui| {
            ui.label(entity);
        });
    }
}

fn inspector_panel(ui: &mut egui::Ui) {
    // Placeholder until component reflection lands.
    ui.heading("Transform");
    egui::Grid::new("transform_grid")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Position");
            ui.label("0.0, 0.0, 0.0");
            ui.end_row();
            ui.label("Rotation");
            ui.label("0.0, 0.0, 0.0");
            ui.end_row();
            ui.label("Scale");
            ui.label("1.0, 1.0, 1.0");
            ui.end_row();
        });
}

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

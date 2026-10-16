//! The inspector's component registry: which component types get
//! auto-generated editing UI, plus the generic [`Reflect`] → egui walker.
//!
//! A registered type contributes a collapsing header per component on the
//! selected entity; the walker renders struct fields recursively (nested
//! structs get collapsing headers) and leaf values through `Any` downcast
//! widgets (`f32` drag, `Vec3` triple drag, `Quat` as Euler-XYZ degrees, …).

use crate::action::field::{FieldValue, ReflectedField};
use moonfield_ecs::{Component, Entity, World};
use moonfield_math::{EulerRot, Quat, Vec3};
use moonfield_reflect::Reflect;
use std::any::TypeId;

pub(crate) mod edits;

type ReadComponent = fn(&World, Entity) -> Option<&dyn Reflect>;
type FieldTarget = fn(Entity, Vec<String>) -> ReflectedField;

/// Type-erased per-component access used by the inspector. `Copy` so the
/// panel can take a snapshot of the registry and release the resource borrow
/// before mutating the world.
#[derive(Clone, Copy)]
pub struct ReflectComponent {
    /// Short display name (last segment of the type path).
    pub display_name: &'static str,
    /// Whether the entity has this component.
    pub has: fn(&World, Entity) -> bool,
    read: ReadComponent,
    field_target: FieldTarget,
    component_type: TypeId,
}

/// The set of component types the inspector can display and edit.
///
/// Stored as a world resource by `EditorPlugin`; game code can register
/// additional component types via
/// `world.get_resource_mut::<InspectorRegistry>().unwrap().register::<T>()`.
#[derive(Default)]
pub struct InspectorRegistry {
    components: Vec<ReflectComponent>,
}

impl InspectorRegistry {
    /// A registry pre-populated with the engine types the editor knows.
    pub fn with_engine_types() -> Self {
        let mut registry = Self::default();
        registry.register::<moonfield_math::Transform>();
        registry.register::<moonfield_camera::Camera>();
        registry.register::<moonfield_render_feature::mesh::MeshRenderer>();
        registry
    }

    /// Register a component type for auto-generated inspector UI.
    pub fn register<T: Component + Reflect>(&mut self) {
        let type_name = std::any::type_name::<T>();
        self.components.push(ReflectComponent {
            display_name: type_name.rsplit("::").next().unwrap_or(type_name),
            has: |world, entity| world.get_component::<T>(entity).is_some(),
            read: |world, entity| {
                world
                    .get_component::<T>(entity)
                    .map(|value| value as &dyn Reflect)
            },
            field_target: ReflectedField::new::<T>,
            component_type: TypeId::of::<T>(),
        });
    }

    /// A snapshot of the registered components (cheap: `Copy` entries).
    pub fn components(&self) -> Vec<ReflectComponent> {
        self.components.clone()
    }
}

/// Render registered components from shared references. Widgets edit owned
/// leaf drafts; only actual edits acquire mutable component access.
pub fn show_components(
    ui: &mut egui::Ui,
    world: &mut World,
    entity: Entity,
    components: &[ReflectComponent],
) {
    for component in components {
        if !(component.has)(world, entity) {
            continue;
        }
        egui::CollapsingHeader::new(component.display_name)
            .default_open(true)
            .id_salt((entity, component.component_type))
            .show(ui, |ui| {
                let Some(value) = (component.read)(world, entity) else {
                    return;
                };
                let mut changes = Vec::new();
                reflect_ui(ui, value, &mut Vec::new(), &mut changes);
                for change in changes {
                    let field = (component.field_target)(entity, change.path);
                    if let Err(error) = edits::submit(
                        world,
                        field,
                        change.value,
                        change.state.changed,
                        change.state.active,
                    ) {
                        ui.colored_label(egui::Color32::RED, error.to_string());
                    }
                }
            });
    }
}

/// Generic field editor for a reflected value: nested structs recurse into
/// collapsing headers, leaves get a widget by `Any` downcast.
fn reflect_ui(
    ui: &mut egui::Ui,
    value: &dyn Reflect,
    path: &mut Vec<&'static str>,
    changes: &mut Vec<FieldEdit>,
) {
    let infos = value.field_infos();
    // Label scratch, reused across this level's fields (the walker runs
    // every frame); nested structs allocate their own on recursion.
    let mut label = String::new();
    for info in infos {
        let Some(field) = value.field(info.name) else {
            continue;
        };
        path.push(info.name);
        if field.field_infos().is_empty() {
            // Leaf: label + widget on one row.
            ui.horizontal(|ui| {
                ui.label(prettify(&mut label, info.name));
                ui.push_id(info.name, |ui| {
                    let Some(mut draft) = FieldValue::capture(field) else {
                        ui.label(format!("(unsupported: {})", field.type_name()));
                        return;
                    };
                    let state = leaf_widget(ui, draft.reflect_mut());
                    if state.changed || state.active {
                        changes.push(FieldEdit {
                            path: path.iter().map(|name| (*name).to_owned()).collect(),
                            value: draft,
                            state,
                        });
                    }
                });
            });
        } else {
            egui::CollapsingHeader::new(prettify(&mut label, info.name))
                .id_salt((ui.id(), info.name))
                .default_open(true)
                .show(ui, |ui| reflect_ui(ui, field, path, changes));
        }
        path.pop();
    }
}

struct FieldEdit {
    path: Vec<String>,
    value: FieldValue,
    state: WidgetState,
}

#[derive(Default)]
struct WidgetState {
    changed: bool,
    active: bool,
}

impl WidgetState {
    fn include(&mut self, response: egui::Response) {
        self.changed |= response.changed();
        self.active |= response.dragged() || response.has_focus();
    }
}

/// `"clear_color"` → `"Clear color"`, written into the caller's scratch
/// buffer instead of allocating a fresh String per field per frame.
fn prettify<'a>(scratch: &'a mut String, field_name: &str) -> &'a str {
    scratch.clear();
    scratch.reserve(field_name.len());
    for (index, ch) in field_name.chars().enumerate() {
        let ch = if ch == '_' { ' ' } else { ch };
        if index == 0 {
            scratch.push(ch.to_ascii_uppercase());
        } else {
            scratch.push(ch);
        }
    }
    scratch
}

/// Leaf-value editing widget, dispatched by type downcast.
fn leaf_widget(ui: &mut egui::Ui, value: &mut dyn Reflect) -> WidgetState {
    let mut state = WidgetState::default();
    let any = value.as_any_mut();
    if let Some(v) = any.downcast_mut::<Vec3>() {
        return vec3_drag(ui, v, 0.1);
    } else if let Some(q) = any.downcast_mut::<Quat>() {
        return quat_euler_drag(ui, q);
    } else if let Some(f) = any.downcast_mut::<f32>() {
        state.include(ui.add(egui::DragValue::new(f).speed(0.1)));
    } else if let Some(f) = any.downcast_mut::<f64>() {
        state.include(ui.add(egui::DragValue::new(f).speed(0.1)));
    } else if let Some(b) = any.downcast_mut::<bool>() {
        state.include(ui.checkbox(b, ""));
        state.active = false;
    } else if let Some(s) = any.downcast_mut::<String>() {
        state.include(ui.text_edit_singleline(s));
    } else if let Some(c) = any.downcast_mut::<[f32; 4]>() {
        return rgba_drag(ui, c);
    } else {
        ui.label(format!("(unsupported: {})", value.type_name()));
    }
    state
}

/// Three inline drag values for a [`Vec3`].
fn vec3_drag(ui: &mut egui::Ui, value: &mut Vec3, speed: f32) -> WidgetState {
    let mut state = WidgetState::default();
    state.include(ui.add(egui::DragValue::new(&mut value.x).speed(speed).prefix("X ")));
    state.include(ui.add(egui::DragValue::new(&mut value.y).speed(speed).prefix("Y ")));
    state.include(ui.add(egui::DragValue::new(&mut value.z).speed(speed).prefix("Z ")));
    state
}

/// A [`Quat`] edited as Euler-XYZ angles in degrees (same UX as the previous
/// hardcoded Transform inspector).
fn quat_euler_drag(ui: &mut egui::Ui, value: &mut Quat) -> WidgetState {
    let (angle_x, angle_y, angle_z) = value.to_euler(EulerRot::XYZ);
    let (mut angle_x, mut angle_y, mut angle_z) = (
        angle_x.to_degrees(),
        angle_y.to_degrees(),
        angle_z.to_degrees(),
    );
    let mut state = WidgetState::default();
    state.include(ui.add(egui::DragValue::new(&mut angle_x).speed(1.0).prefix("X ")));
    state.include(ui.add(egui::DragValue::new(&mut angle_y).speed(1.0).prefix("Y ")));
    state.include(ui.add(egui::DragValue::new(&mut angle_z).speed(1.0).prefix("Z ")));
    if state.changed {
        *value = Quat::from_euler(
            EulerRot::XYZ,
            angle_x.to_radians(),
            angle_y.to_radians(),
            angle_z.to_radians(),
        );
    }
    state
}

/// Four inline drag values for an RGBA color stored as `[f32; 4]`.
fn rgba_drag(ui: &mut egui::Ui, color: &mut [f32; 4]) -> WidgetState {
    let mut state = WidgetState::default();
    for (index, label) in ["R ", "G ", "B ", "A "].iter().enumerate() {
        state.include(
            ui.add(
                egui::DragValue::new(&mut color[index])
                    .speed(0.01)
                    .prefix(*label),
            ),
        );
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_math::Transform;

    fn engine_components() -> Vec<ReflectComponent> {
        InspectorRegistry::with_engine_types().components()
    }

    #[test]
    fn test_registry_with_engine_types_shows_transform() {
        let mut world = World::new();
        let entity = world.spawn((Transform::from_xyz(1.0, 2.0, 3.0),));

        let components = engine_components();
        let transform_entry = components
            .iter()
            .find(|c| c.display_name == "Transform")
            .expect("Transform registered");

        assert!((transform_entry.has)(&world, entity));
        let field = (transform_entry.field_target)(entity, vec!["translation".into()]);
        crate::action::perform(
            &mut world,
            crate::action::EditorAction::SetField {
                field,
                value: FieldValue::Vector(Vec3::new(9.0, 2.0, 3.0)),
            },
        )
        .unwrap();
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::new(9.0, 2.0, 3.0)
        );
    }

    #[test]
    fn test_registry_skips_absent_components() {
        let mut world = World::new();
        let entity = world.spawn(());
        for component in engine_components() {
            assert!(!(component.has)(&world, entity));
        }
    }

    #[test]
    fn test_idle_inspector_does_not_mark_changed_or_create_history() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        world.increment_change_tick();
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            show_components(ui, &mut world, entity, &engine_components());
        })
        .drop_without_applying_deltas();
        assert!(
            !world
                .get_component_ref::<Transform>(entity)
                .unwrap()
                .is_changed()
        );
        assert!(crate::action::undo(&mut world).is_none());
    }

    #[derive(Reflect)]
    struct Toggle {
        enabled: bool,
    }

    fn reflect_frame<T: Component + Reflect>(
        ctx: &egui::Context,
        world: &mut World,
        entity: Entity,
        input: egui::RawInput,
    ) -> egui::Rect {
        let mut rect = egui::Rect::NOTHING;
        ctx.run_ui(input, |ui| {
            edits::begin_frame(world);
            let mut changes = Vec::new();
            reflect_ui(
                ui,
                world.get_component::<T>(entity).unwrap(),
                &mut Vec::new(),
                &mut changes,
            );
            rect = ui.min_rect();
            for change in changes {
                edits::submit(
                    world,
                    ReflectedField::new::<T>(entity, change.path),
                    change.value,
                    change.state.changed,
                    change.state.active,
                )
                .unwrap();
            }
            edits::finish_frame(world, Some(entity));
        })
        .drop_without_applying_deltas();
        rect
    }

    fn pointer_input(position: egui::Pos2, pressed: bool) -> egui::RawInput {
        egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn test_headless_checkbox_edit_submits_and_round_trips_history() {
        let ctx = egui::Context::default();
        let mut world = World::new();
        let entity = world.spawn((Toggle { enabled: false },));
        world.increment_change_tick();
        let position = reflect_frame::<Toggle>(&ctx, &mut world, entity, egui::RawInput::default())
            .right_center()
            - egui::vec2(8.0, 0.0);
        for pressed in [true, false] {
            reflect_frame::<Toggle>(&ctx, &mut world, entity, pointer_input(position, pressed));
        }
        assert!(world.get_component::<Toggle>(entity).unwrap().enabled);
        assert!(
            world
                .get_component_ref::<Toggle>(entity)
                .unwrap()
                .is_changed()
        );
        crate::action::undo(&mut world).unwrap().unwrap();
        assert!(!world.get_component::<Toggle>(entity).unwrap().enabled);
        assert!(crate::action::undo(&mut world).is_none());
        crate::action::redo(&mut world).unwrap().unwrap();
        assert!(world.get_component::<Toggle>(entity).unwrap().enabled);
    }

    #[derive(Reflect)]
    struct TextField {
        label: String,
    }

    #[test]
    fn test_headless_text_edit_finishes_on_focus_loss() {
        let ctx = egui::Context::default();
        let mut world = World::new();
        let entity = world.spawn((TextField {
            label: String::new(),
        },));
        let position =
            reflect_frame::<TextField>(&ctx, &mut world, entity, egui::RawInput::default())
                .right_center()
                - egui::vec2(8.0, 0.0);
        for pressed in [true, false] {
            reflect_frame::<TextField>(&ctx, &mut world, entity, pointer_input(position, pressed));
        }
        for text in ["a", "b"] {
            reflect_frame::<TextField>(
                &ctx,
                &mut world,
                entity,
                egui::RawInput {
                    events: vec![egui::Event::Text(text.into())],
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            world.get_component::<TextField>(entity).unwrap().label,
            "ab"
        );
        assert!(
            world
                .get_resource::<crate::action::EditorHistory>()
                .is_none()
        );
        for pressed in [true, false] {
            reflect_frame::<TextField>(
                &ctx,
                &mut world,
                entity,
                pointer_input(egui::pos2(600.0, 600.0), pressed),
            );
        }
        assert!(
            world
                .get_resource::<crate::action::EditorHistory>()
                .is_some()
        );
        crate::action::undo(&mut world).unwrap().unwrap();
        assert!(
            world
                .get_component::<TextField>(entity)
                .unwrap()
                .label
                .is_empty()
        );
        assert!(crate::action::undo(&mut world).is_none());
        crate::action::redo(&mut world).unwrap().unwrap();
        assert_eq!(
            world.get_component::<TextField>(entity).unwrap().label,
            "ab"
        );
    }

    #[derive(Reflect)]
    struct ScalarField {
        value: f32,
    }

    #[test]
    fn test_headless_drag_records_one_unit_on_release() {
        let ctx = egui::Context::default();
        let mut world = World::new();
        let entity = world.spawn((ScalarField { value: 0.0 },));
        let position =
            reflect_frame::<ScalarField>(&ctx, &mut world, entity, egui::RawInput::default())
                .right_center()
                - egui::vec2(8.0, 0.0);
        reflect_frame::<ScalarField>(&ctx, &mut world, entity, pointer_input(position, true));
        for distance in [50.0, 100.0] {
            reflect_frame::<ScalarField>(
                &ctx,
                &mut world,
                entity,
                egui::RawInput {
                    events: vec![egui::Event::PointerMoved(
                        position + egui::vec2(distance, 0.0),
                    )],
                    ..Default::default()
                },
            );
        }
        let after = world.get_component::<ScalarField>(entity).unwrap().value;
        assert!(after > 0.0);
        assert!(
            world
                .get_resource::<crate::action::EditorHistory>()
                .is_none()
        );
        reflect_frame::<ScalarField>(
            &ctx,
            &mut world,
            entity,
            pointer_input(position + egui::vec2(100.0, 0.0), false),
        );
        assert!(
            world
                .get_resource::<crate::action::EditorHistory>()
                .is_some()
        );
        crate::action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world.get_component::<ScalarField>(entity).unwrap().value,
            0.0
        );
        assert!(crate::action::undo(&mut world).is_none());
        crate::action::redo(&mut world).unwrap().unwrap();
        assert_eq!(
            world.get_component::<ScalarField>(entity).unwrap().value,
            after
        );
    }

    /// The generic walker drives egui without a display: rendering a
    /// reflected Transform must not panic and must not mutate the value.
    #[test]
    fn test_reflect_ui_runs_headless() {
        let transform = Transform::from_xyz(1.0, 0.0, 0.0);
        let ctx = egui::Context::default();
        // The output's texture deltas are irrelevant here; epaint 0.36
        // debug-asserts that a dropped `TexturesDelta` was fully applied.
        ctx.run_ui(egui::RawInput::default(), |ui| {
            reflect_ui(ui, &transform, &mut Vec::new(), &mut Vec::new());
        })
        .drop_without_applying_deltas();
        assert_eq!(transform.translation, Vec3::new(1.0, 0.0, 0.0));
    }
}

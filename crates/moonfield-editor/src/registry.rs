//! The inspector's component registry: which component types get
//! auto-generated editing UI, plus the generic [`Reflect`] → egui walker.
//!
//! A registered type contributes a collapsing header per component on the
//! selected entity; the walker renders struct fields recursively (nested
//! structs get collapsing headers) and leaf values through `Any` downcast
//! widgets (`f32` drag, `Vec3` triple drag, `Quat` as Euler-XYZ degrees, …).

use moonfield_ecs::{Component, Entity, World};
use moonfield_math::{EulerRot, Quat, Vec3};
use moonfield_reflect::Reflect;

/// Type-erased component editor callback: runs `f` with the component as
/// `&mut dyn Reflect` if present on the entity.
pub type WithComponentMut = fn(&mut World, Entity, &mut dyn FnMut(&mut dyn Reflect));

/// Type-erased per-component access used by the inspector. `Copy` so the
/// panel can take a snapshot of the registry and release the resource borrow
/// before mutating the world.
#[derive(Clone, Copy)]
pub struct ReflectComponent {
    /// Short display name (last segment of the type path).
    pub display_name: &'static str,
    /// Whether the entity has this component.
    pub has: fn(&World, Entity) -> bool,
    /// Runs the editor callback with the component, if present.
    pub with_mut: WithComponentMut,
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
        registry.register::<moonfield_render::Camera>();
        registry.register::<moonfield_renderer::mesh::MeshRenderer>();
        registry
    }

    /// Register a component type for auto-generated inspector UI.
    pub fn register<T: Component + Reflect>(&mut self) {
        let type_name = std::any::type_name::<T>();
        self.components.push(ReflectComponent {
            display_name: type_name.rsplit("::").next().unwrap_or(type_name),
            has: |world, entity| world.get_component::<T>(entity).is_some(),
            with_mut: |world, entity, f| {
                if let Some(mut component) = world.get_component_mut::<T>(entity) {
                    f(&mut *component);
                }
            },
        });
    }

    /// A snapshot of the registered components (cheap: `Copy` entries).
    pub fn components(&self) -> Vec<ReflectComponent> {
        self.components.clone()
    }
}

/// Render the editing UI for every registered component present on `entity`.
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
            .show(ui, |ui| {
                (component.with_mut)(world, entity, &mut |value| reflect_ui(ui, value));
            });
    }
}

/// Generic field editor for a reflected value: nested structs recurse into
/// collapsing headers, leaves get a widget by `Any` downcast.
pub fn reflect_ui(ui: &mut egui::Ui, value: &mut dyn Reflect) {
    let infos = value.field_infos();
    for info in infos {
        let Some(field) = value.field_mut(info.name) else {
            continue;
        };
        if field.field_infos().is_empty() {
            // Leaf: label + widget on one row.
            ui.horizontal(|ui| {
                ui.label(prettify(info.name));
                leaf_widget(ui, field);
            });
        } else {
            egui::CollapsingHeader::new(prettify(info.name))
                .id_salt((ui.id(), info.name))
                .default_open(true)
                .show(ui, |ui| reflect_ui(ui, field));
        }
    }
}

/// `"clear_color"` → `"Clear color"`.
fn prettify(field_name: &str) -> String {
    let mut s = field_name.replace('_', " ");
    if let Some(first) = s.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    s
}

/// Leaf-value editing widget, dispatched by type downcast.
fn leaf_widget(ui: &mut egui::Ui, value: &mut dyn Reflect) {
    let any = value.as_any_mut();
    if let Some(v) = any.downcast_mut::<Vec3>() {
        vec3_drag(ui, v, 0.1);
    } else if let Some(q) = any.downcast_mut::<Quat>() {
        quat_euler_drag(ui, q);
    } else if let Some(f) = any.downcast_mut::<f32>() {
        ui.add(egui::DragValue::new(f).speed(0.1));
    } else if let Some(f) = any.downcast_mut::<f64>() {
        ui.add(egui::DragValue::new(f).speed(0.1));
    } else if let Some(b) = any.downcast_mut::<bool>() {
        ui.checkbox(b, "");
    } else if let Some(s) = any.downcast_mut::<String>() {
        ui.text_edit_singleline(s);
    } else if let Some(c) = any.downcast_mut::<[f32; 4]>() {
        rgba_drag(ui, c);
    } else {
        ui.label(format!("(unsupported: {})", value.type_name()));
    }
}

/// Three inline drag values for a [`Vec3`].
fn vec3_drag(ui: &mut egui::Ui, v: &mut Vec3, speed: f32) {
    ui.add(egui::DragValue::new(&mut v.x).speed(speed).prefix("X "));
    ui.add(egui::DragValue::new(&mut v.y).speed(speed).prefix("Y "));
    ui.add(egui::DragValue::new(&mut v.z).speed(speed).prefix("Z "));
}

/// A [`Quat`] edited as Euler-XYZ angles in degrees (same UX as the previous
/// hardcoded Transform inspector).
fn quat_euler_drag(ui: &mut egui::Ui, q: &mut Quat) {
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    let (mut x, mut y, mut z) = (x.to_degrees(), y.to_degrees(), z.to_degrees());
    let mut changed = false;
    changed |= ui
        .add(egui::DragValue::new(&mut x).speed(1.0).prefix("X "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut y).speed(1.0).prefix("Y "))
        .changed();
    changed |= ui
        .add(egui::DragValue::new(&mut z).speed(1.0).prefix("Z "))
        .changed();
    if changed {
        *q = Quat::from_euler(
            EulerRot::XYZ,
            x.to_radians(),
            y.to_radians(),
            z.to_radians(),
        );
    }
}

/// Four inline drag values for an RGBA color stored as `[f32; 4]`.
fn rgba_drag(ui: &mut egui::Ui, c: &mut [f32; 4]) {
    for (i, label) in ["R ", "G ", "B ", "A "].iter().enumerate() {
        ui.add(egui::DragValue::new(&mut c[i]).speed(0.01).prefix(*label));
    }
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
        // Write through the type-erased accessor.
        (transform_entry.with_mut)(&mut world, entity, &mut |value| {
            let translation = value.field_mut("translation").unwrap();
            let v = translation.as_any_mut().downcast_mut::<Vec3>().unwrap();
            v.x = 9.0;
        });
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

    /// The generic walker drives egui without a display: rendering a
    /// reflected Transform must not panic and must not mutate the value.
    #[test]
    fn test_reflect_ui_runs_headless() {
        let mut transform = Transform::from_xyz(1.0, 0.0, 0.0);
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                reflect_ui(ui, &mut transform);
            });
        });
        assert_eq!(transform.translation, Vec3::new(1.0, 0.0, 0.0));
    }
}

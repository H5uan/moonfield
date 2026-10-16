use std::any::TypeId;

use moonfield_ecs::{Component, Entity, World};
use moonfield_math::{Quat, Vec3};
use moonfield_reflect::Reflect;

use super::ActionError;

macro_rules! field_values {
    ($($variant:ident: $value_type:ty),+ $(,)?) => {
        /// Owned values for the Inspector's supported leaf widgets.
        #[derive(Debug, Clone)]
        pub enum FieldValue {
            $($variant($value_type)),+
        }

        impl FieldValue {
            pub(crate) fn capture(value: &dyn Reflect) -> Option<Self> {
                $(if let Some(value) = value.as_any().downcast_ref::<$value_type>() {
                    return Some(Self::$variant(value.clone()));
                })+
                None
            }

            fn assign(&self, value: &mut dyn Reflect) {
                match self {
                    $(Self::$variant(source) => {
                        *value.as_any_mut().downcast_mut::<$value_type>()
                            .expect("field type validated before mutation") = source.clone();
                    })+
                }
            }

            pub(crate) fn reflect_mut(&mut self) -> &mut dyn Reflect {
                match self {
                    $(Self::$variant(value) => value),+
                }
            }
        }
    };
}

field_values! {
    Scalar: f32,
    Double: f64,
    Boolean: bool,
    Text: String,
    Vector: Vec3,
    Rotation: Quat,
    Color: [f32; 4],
}

impl PartialEq for FieldValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Scalar(left), Self::Scalar(right)) => left.to_bits() == right.to_bits(),
            (Self::Double(left), Self::Double(right)) => left.to_bits() == right.to_bits(),
            (Self::Boolean(left), Self::Boolean(right)) => left == right,
            (Self::Text(left), Self::Text(right)) => left == right,
            (Self::Vector(left), Self::Vector(right)) => {
                left.to_array().map(f32::to_bits) == right.to_array().map(f32::to_bits)
            }
            (Self::Rotation(left), Self::Rotation(right)) => {
                left.to_array().map(f32::to_bits) == right.to_array().map(f32::to_bits)
            }
            (Self::Color(left), Self::Color(right)) => {
                left.map(f32::to_bits) == right.map(f32::to_bits)
            }
            _ => false,
        }
    }
}

type ReadComponent = fn(&World, Entity) -> Option<&dyn Reflect>;
type WriteField = fn(&mut World, Entity, &[String], &FieldValue);

/// An entity, concrete component type, and named-field path.
/// Access callbacks resolve the target on every operation; no ECS pointers
/// or component snapshots are retained in history.
#[derive(Debug, Clone)]
pub struct ReflectedField {
    pub entity: Entity,
    component: TypeId,
    component_name: &'static str,
    path: Vec<String>,
    read_component: ReadComponent,
    write_field: WriteField,
}

impl ReflectedField {
    /// An empty path targets a component that is itself a supported leaf.
    pub fn new<T: Component + Reflect>(entity: Entity, path: Vec<String>) -> Self {
        Self {
            entity,
            component: TypeId::of::<T>(),
            component_name: std::any::type_name::<T>(),
            path,
            read_component: |world, entity| {
                world
                    .get_component::<T>(entity)
                    .map(|value| value as &dyn Reflect)
            },
            write_field: |world, entity, path, value| {
                let mut component = world
                    .get_component_mut::<T>(entity)
                    .expect("component validated before mutation");
                let mut field: &mut dyn Reflect = &mut *component;
                for name in path {
                    field = field
                        .field_mut(name)
                        .expect("field validated before mutation");
                }
                value.assign(field);
            },
        }
    }

    /// Read a supported leaf, rejecting stale entities and missing fields.
    pub fn read(&self, world: &World) -> Result<FieldValue, ActionError> {
        if !world.contains(self.entity) {
            return Err(ActionError::MissingEntity(self.entity));
        }
        let mut field = (self.read_component)(world, self.entity)
            .ok_or_else(|| self.error("component is missing"))?;
        for name in &self.path {
            field = field
                .field(name)
                .ok_or_else(|| self.error("field is missing"))?;
        }
        FieldValue::capture(field).ok_or_else(|| self.error("unsupported field type"))
    }

    pub(crate) fn validate(
        &self,
        world: &World,
        value: &FieldValue,
    ) -> Result<FieldValue, ActionError> {
        let before = self.read(world)?;
        if std::mem::discriminant(&before) != std::mem::discriminant(value) {
            return Err(self.error("field value type does not match"));
        }
        Ok(before)
    }

    pub(crate) fn write(
        &self,
        world: &mut World,
        value: &FieldValue,
    ) -> Result<Option<FieldValue>, ActionError> {
        let before = self.validate(world, value)?;
        if before == *value {
            return Ok(None);
        }
        (self.write_field)(world, self.entity, &self.path, value);
        Ok(Some(before))
    }

    pub(crate) fn same_target(&self, other: &Self) -> bool {
        self.entity == other.entity && self.component == other.component && self.path == other.path
    }

    fn error(&self, reason: &str) -> ActionError {
        ActionError::ReflectedField(format!(
            "{} on {:?}, {}: {reason}",
            self.component_name,
            self.entity,
            self.path.join(".")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{self, EditorAction, EditorUndo};
    use moonfield_math::Transform;

    fn translation(entity: Entity) -> ReflectedField {
        ReflectedField::new::<Transform>(entity, vec!["translation".into()])
    }

    #[derive(Reflect)]
    struct Nested {
        label: String,
    }

    #[derive(Reflect)]
    struct CustomComponent {
        nested: Nested,
        unsupported: u32,
        #[reflect(ignore)]
        ignored: String,
    }

    #[test]
    fn test_nested_nonclone_component_preserves_other_fields() {
        let mut world = World::new();
        let entity = world.spawn((CustomComponent {
            nested: Nested {
                label: "before".into(),
            },
            unsupported: 42,
            ignored: "untouched".into(),
        },));
        let field =
            ReflectedField::new::<CustomComponent>(entity, vec!["nested".into(), "label".into()]);
        action::perform(
            &mut world,
            EditorAction::SetField {
                field: field.clone(),
                value: FieldValue::Text("after".into()),
            },
        )
        .unwrap();
        assert_eq!(
            field.read(&world).unwrap(),
            FieldValue::Text("after".into())
        );
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            field.read(&world).unwrap(),
            FieldValue::Text("before".into())
        );
        action::redo(&mut world).unwrap().unwrap();
        let component = world.get_component::<CustomComponent>(entity).unwrap();
        assert_eq!(component.unsupported, 42);
        assert_eq!(component.ignored, "untouched");
        world.increment_change_tick();
        for name in ["unsupported", "ignored"] {
            assert!(
                ReflectedField::new::<CustomComponent>(entity, vec![name.into()])
                    .write(&mut world, &FieldValue::Scalar(1.0))
                    .is_err()
            );
        }
        assert!(
            !world
                .get_component_ref::<CustomComponent>(entity)
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn test_stale_entity_does_not_edit_reused_slot() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        let field = translation(entity);
        world.despawn(entity).unwrap();
        let replacement = world.spawn((Transform::IDENTITY,));
        world.increment_change_tick();
        assert!(matches!(
            field.write(&mut world, &FieldValue::Vector(Vec3::X)),
            Err(ActionError::MissingEntity(_))
        ));
        assert!(
            !world
                .get_component_ref::<Transform>(replacement)
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn test_nan_noop_uses_storage_equality() {
        let mut world = World::new();
        let entity = world.spawn((f32::NAN,));
        world.increment_change_tick();
        let field = ReflectedField::new::<f32>(entity, Vec::new());
        assert!(
            field
                .write(&mut world, &FieldValue::Scalar(f32::NAN))
                .unwrap()
                .is_none()
        );
        assert!(!world.get_component_ref::<f32>(entity).unwrap().is_changed());
    }

    #[test]
    fn test_field_edit_marks_changed_and_round_trips_history() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        world.increment_change_tick();
        let field = translation(entity);
        action::perform(
            &mut world,
            EditorAction::SetField {
                field: field.clone(),
                value: FieldValue::Vector(Vec3::X),
            },
        )
        .unwrap();
        assert!(
            world
                .get_component_ref::<Transform>(entity)
                .unwrap()
                .is_changed()
        );
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(field.read(&world).unwrap(), FieldValue::Vector(Vec3::ZERO));
        action::redo(&mut world).unwrap().unwrap();
        assert_eq!(field.read(&world).unwrap(), FieldValue::Vector(Vec3::X));
    }

    #[test]
    fn test_noop_keeps_redo_and_change_ticks() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        let field = translation(entity);
        action::perform(
            &mut world,
            EditorAction::SetField {
                field: field.clone(),
                value: FieldValue::Vector(Vec3::X),
            },
        )
        .unwrap();
        action::undo(&mut world).unwrap().unwrap();
        world.increment_change_tick();
        let outcome = action::perform(
            &mut world,
            EditorAction::SetField {
                field,
                value: FieldValue::Vector(Vec3::ZERO),
            },
        )
        .unwrap();
        assert!(outcome.undo.is_none());
        assert!(
            !world
                .get_component_ref::<Transform>(entity)
                .unwrap()
                .is_changed()
        );
        action::redo(&mut world).unwrap().unwrap();
    }

    #[test]
    fn test_invalid_field_or_type_does_not_mark_changed() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        world.increment_change_tick();
        for (field, value) in [
            (translation(entity), FieldValue::Scalar(1.0)),
            (
                ReflectedField::new::<Transform>(entity, vec!["missing".into()]),
                FieldValue::Vector(Vec3::X),
            ),
        ] {
            assert!(action::execute(&mut world, &EditorAction::SetField { field, value }).is_err());
        }
        assert!(
            !world
                .get_component_ref::<Transform>(entity)
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn test_missing_component_undo_is_retryable() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        let field = translation(entity);
        action::perform(
            &mut world,
            EditorAction::SetField {
                field,
                value: FieldValue::Vector(Vec3::X),
            },
        )
        .unwrap();
        world.remove_component::<Transform>(entity).unwrap();
        assert!(action::undo(&mut world).unwrap().is_err());
        world
            .insert_component(entity, Transform::from_translation(Vec3::X))
            .unwrap();
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
    }

    #[test]
    fn test_undo_batch_validates_all_fields_before_mutation() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        world.increment_change_tick();
        let undo = EditorUndo::Batch(vec![
            EditorUndo::SetField {
                field: translation(entity),
                value: FieldValue::Vector(Vec3::X),
            },
            EditorUndo::SetField {
                field: translation(entity),
                value: FieldValue::Scalar(3.0),
            },
        ]);
        assert!(undo.apply(&mut world).is_err());
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
        assert!(
            !world
                .get_component_ref::<Transform>(entity)
                .unwrap()
                .is_changed()
        );
    }
}

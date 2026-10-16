use moonfield_ecs::{Entity, World};

use crate::action::field::{FieldValue, ReflectedField};
use crate::action::{self, ActionError, EditorAction, EditorUndo};

struct PendingEdit {
    field: ReflectedField,
    before: FieldValue,
    after: FieldValue,
}

#[derive(Default)]
struct InspectorEdits {
    pending: Option<PendingEdit>,
    active_this_frame: bool,
}

pub(crate) fn begin_frame(world: &mut World) {
    if let Some(mut edits) = world.get_resource_mut::<InspectorEdits>() {
        edits.active_this_frame = false;
    }
}

pub(crate) fn finish_frame(world: &mut World, selection: Option<Entity>) {
    if world.get_resource::<InspectorEdits>().is_some_and(|edits| {
        !edits.active_this_frame
            || edits
                .pending
                .as_ref()
                .is_some_and(|pending| Some(pending.field.entity) != selection)
    }) {
        flush(world);
    }
}

pub(crate) fn flush(world: &mut World) {
    let pending = world
        .get_resource_mut::<InspectorEdits>()
        .and_then(|mut edits| edits.pending.take());
    if let Some(pending) = pending
        && pending.before != pending.after
    {
        action::record_completed(
            world,
            EditorAction::SetField {
                field: pending.field.clone(),
                value: pending.after,
            },
            EditorUndo::SetField {
                field: pending.field,
                value: pending.before,
            },
        );
    }
}

pub(super) fn submit(
    world: &mut World,
    field: ReflectedField,
    value: FieldValue,
    changed: bool,
    active: bool,
) -> Result<(), ActionError> {
    if !changed {
        update_activity(world, &field, active);
        return Ok(());
    }
    let before = field.validate(world, &value)?;
    if before == value {
        update_activity(world, &field, active);
        return Ok(());
    }
    let continues = world.get_resource::<InspectorEdits>().is_some_and(|edits| {
        edits
            .pending
            .as_ref()
            .is_some_and(|pending| pending.field.same_target(&field) && pending.after == before)
    });
    if !continues {
        flush(world);
    }
    action::execute(
        world,
        &EditorAction::SetField {
            field: field.clone(),
            value: value.clone(),
        },
    )?;
    world.init_resource::<InspectorEdits>();
    let mut edits = world.get_resource_mut::<InspectorEdits>().unwrap();
    if let Some(pending) = edits.pending.as_mut() {
        pending.after = value;
    } else {
        edits.pending = Some(PendingEdit {
            field,
            before,
            after: value,
        });
    }
    edits.active_this_frame = active;
    drop(edits);
    if !active {
        flush(world);
    }
    Ok(())
}

fn update_activity(world: &mut World, field: &ReflectedField, active: bool) {
    let matches_pending = world.get_resource::<InspectorEdits>().is_some_and(|edits| {
        edits
            .pending
            .as_ref()
            .is_some_and(|pending| pending.field.same_target(field))
    });
    if matches_pending {
        if active {
            world
                .get_resource_mut::<InspectorEdits>()
                .unwrap()
                .active_this_frame = true;
        } else {
            flush(world);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action;
    use moonfield_math::{Transform, Vec3};

    fn field(entity: moonfield_ecs::Entity) -> ReflectedField {
        ReflectedField::new::<Transform>(entity, vec!["translation".into()])
    }

    #[test]
    fn test_continuous_edit_records_one_undo_unit() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        for value in [Vec3::X, Vec3::Y, Vec3::Z] {
            begin_frame(&mut world);
            submit(
                &mut world,
                field(entity),
                FieldValue::Vector(value),
                true,
                true,
            )
            .unwrap();
            finish_frame(&mut world, Some(entity));
        }
        begin_frame(&mut world);
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::Z),
            false,
            false,
        )
        .unwrap();
        finish_frame(&mut world, Some(entity));
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
        assert!(action::undo(&mut world).is_none());
        action::redo(&mut world).unwrap().unwrap();
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::Z
        );
    }

    #[test]
    fn test_hidden_widget_finishes_edit() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        begin_frame(&mut world);
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::X),
            true,
            true,
        )
        .unwrap();
        finish_frame(&mut world, Some(entity));
        begin_frame(&mut world);
        finish_frame(&mut world, Some(entity));
        assert!(
            world
                .get_resource::<InspectorEdits>()
                .unwrap()
                .pending
                .is_none()
        );
        action::undo(&mut world).unwrap().unwrap();
    }

    #[test]
    fn test_undo_flushes_active_gesture_first() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::X),
            true,
            true,
        )
        .unwrap();
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
        assert!(action::undo(&mut world).is_none());
    }

    #[test]
    fn test_return_to_initial_value_has_no_history() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::X),
            true,
            true,
        )
        .unwrap();
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::ZERO),
            true,
            false,
        )
        .unwrap();
        assert!(action::undo(&mut world).is_none());
    }

    #[test]
    fn test_selection_change_finishes_active_edit() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        begin_frame(&mut world);
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::X),
            true,
            true,
        )
        .unwrap();
        finish_frame(&mut world, None);
        assert!(
            world
                .get_resource::<InspectorEdits>()
                .unwrap()
                .pending
                .is_none()
        );
        action::undo(&mut world).unwrap().unwrap();
    }

    #[test]
    fn test_active_noop_does_not_split_gesture() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::X),
            true,
            true,
        )
        .unwrap();
        begin_frame(&mut world);
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::X),
            true,
            true,
        )
        .unwrap();
        finish_frame(&mut world, Some(entity));
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::Y),
            true,
            false,
        )
        .unwrap();
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
        assert!(action::undo(&mut world).is_none());
    }

    #[test]
    fn test_action_flushes_pending_edit_in_history_order() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        let other = world.spawn((Transform::IDENTITY,));
        submit(
            &mut world,
            field(entity),
            FieldValue::Vector(Vec3::X),
            true,
            true,
        )
        .unwrap();
        action::perform(
            &mut world,
            EditorAction::SetTransform {
                entity: other,
                transform: Transform::from_xyz(2.0, 0.0, 0.0),
            },
        )
        .unwrap();
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world.get_component::<Transform>(other).unwrap().translation,
            Vec3::ZERO
        );
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::X
        );
        action::undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world
                .get_component::<Transform>(entity)
                .unwrap()
                .translation,
            Vec3::ZERO
        );
    }
}

use moonfield_ecs::{Entity, World};
use moonfield_math::Transform;

use super::{ActionError, ActionOutcome, EditorAction, execute};

/// Compensation produced by a successful editor action.
///
/// Entity cleanup is distinct from user-requested deletion: a reversible
/// batch may produce cleanup inverses without accepting irreversible edits.
#[derive(Debug, Clone)]
pub enum EditorUndo {
    /// Restore an entity's local transform.
    SetTransform {
        entity: Entity,
        transform: Transform,
    },
    /// Remove the roots created by an import or scene load.
    DeleteEntities(Vec<Entity>),
    /// Apply compensations in their stored order.
    Batch(Vec<EditorUndo>),
}

impl EditorUndo {
    /// Apply compensation without recording history. Entity and component
    /// validation completes before any mutation, including for nested batches.
    /// Component hook and template side effects are outside this guarantee.
    pub fn apply(&self, world: &mut World) -> Result<ActionOutcome, ActionError> {
        self.validate(world)?;
        match self {
            Self::SetTransform { entity, transform } => execute(
                world,
                &EditorAction::SetTransform {
                    entity: *entity,
                    transform: *transform,
                },
            ),
            Self::DeleteEntities(entities) => {
                for &entity in entities {
                    if world.contains(entity) {
                        world
                            .despawn(entity)
                            .map_err(|_| ActionError::MissingEntity(entity))?;
                    }
                }
                Ok(ActionOutcome {
                    message: format!("Removed {} created roots", entities.len()),
                    entity: None,
                    undo: None,
                })
            }
            Self::Batch(actions) => {
                let mut entity = None;
                for action in actions {
                    entity = action.apply(world)?.entity.or(entity);
                }
                Ok(ActionOutcome {
                    message: format!("Undid {} actions", actions.len()),
                    entity,
                    undo: None,
                })
            }
        }
    }

    fn validate(&self, world: &World) -> Result<(), ActionError> {
        match self {
            Self::SetTransform { entity, .. } => {
                if !world.contains(*entity) {
                    return Err(ActionError::MissingEntity(*entity));
                }
                if world.get_component::<Transform>(*entity).is_none() {
                    return Err(ActionError::MissingTransform(*entity));
                }
            }
            Self::DeleteEntities(entities) => {
                for &entity in entities {
                    if !world.contains(entity) {
                        return Err(ActionError::MissingEntity(entity));
                    }
                }
            }
            Self::Batch(actions) => {
                for action in actions {
                    action.validate(world)?;
                }
            }
        }
        Ok(())
    }
}

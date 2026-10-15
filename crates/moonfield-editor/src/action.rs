//! Editor actions: the single execution interface for editor operations.
//!
//! UI, keyboard shortcuts, and automation all route through
//! [`EditorAction`] and [`perform`]: this module owns validation, results,
//! and the undo record ([`EditorHistory`]); UI code owns presentation
//! (status text, selection). The render-frame handoff (egui frame
//! extraction, viewport texture upload) is not an action and stays outside
//! this interface.

use std::path::PathBuf;

use moonfield_ecs::{Entity, World};
use moonfield_math::Transform;
use moonfield_scene::SceneRegistry;

mod undo;
pub use undo::EditorUndo;

/// One editor operation.
#[derive(Debug, Clone)]
pub enum EditorAction {
    /// Import an asset file, spawning its entity (mesh or splat cloud, by
    /// content).
    ImportAsset {
        /// The asset path (`.gltf`/`.glb`).
        path: PathBuf,
    },
    /// Load a scene document, spawning its roots.
    LoadScene {
        /// The `.gltf` scene path.
        path: PathBuf,
    },
    /// Save the world's registered entities to a scene document.
    SaveScene {
        /// The `.gltf` scene path.
        path: PathBuf,
    },
    /// Set an entity's local transform.
    SetTransform {
        /// The entity to move.
        entity: Entity,
        /// The new local transform.
        transform: Transform,
    },
    /// Despawn an entity (and its linked children). Not reversible.
    DeleteEntity {
        /// The entity to despawn.
        entity: Entity,
    },
    /// Execute reversible actions in order. Deletion and saving are rejected
    /// before any step executes, including inside nested batches. Completed
    /// steps are compensated on failure; compensation errors are reported.
    Batch(Vec<EditorAction>),
}

/// Errors produced by executing an [`EditorAction`].
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    /// The asset import failed (I/O or parse).
    #[error("asset import failed: {0}")]
    Import(String),
    /// The scene save/load failed.
    #[error("scene I/O failed: {0}")]
    Scene(#[from] moonfield_scene::SceneError),
    /// Scene actions need the world's `SceneRegistry` resource.
    #[error("the SceneRegistry resource is missing")]
    MissingSceneRegistry,
    /// The action names an entity that does not exist.
    #[error("entity {0:?} does not exist")]
    MissingEntity(Entity),
    /// A transform edit names an entity without a `Transform` component.
    #[error("entity {0:?} has no Transform")]
    MissingTransform(Entity),
    /// A reversible batch contains an irreversible operation.
    #[error("{0} is irreversible and cannot be included in a batch")]
    IrreversibleBatch(&'static str),
    /// The operation failed and one or more compensations also failed.
    #[error("{cause}; rollback failed: {errors:?}")]
    Rollback {
        cause: Box<ActionError>,
        errors: Vec<ActionError>,
    },
}

/// The result of a successfully executed action.
#[derive(Debug)]
pub struct ActionOutcome {
    /// Human-readable summary for status lines.
    pub message: String,
    /// The entity the action primarily affected, when there is one; UIs
    /// follow it with their selection.
    pub entity: Option<Entity>,
    /// The inverse action, when the action is reversible.
    pub undo: Option<EditorUndo>,
}

/// Execute `action` against `world` without touching the history (the
/// history interface below wraps this). Validation happens here: missing
/// entities, missing components, and missing resources are errors, not
/// panics.
pub fn execute(world: &mut World, action: &EditorAction) -> Result<ActionOutcome, ActionError> {
    match action {
        EditorAction::ImportAsset { path } => {
            let entity = crate::scene_io::load_asset(world, path).map_err(ActionError::Import)?;
            Ok(ActionOutcome {
                message: format!("Loaded {}", path.display()),
                entity: Some(entity),
                undo: Some(EditorUndo::DeleteEntities(vec![entity])),
            })
        }
        EditorAction::DeleteEntity { entity } => {
            if !world.contains(*entity) {
                return Err(ActionError::MissingEntity(*entity));
            }
            // Hierarchy links cascade to children (linked spawn).
            let _ = world.despawn(*entity);
            Ok(ActionOutcome {
                message: format!("Deleted {entity:?}"),
                entity: None,
                undo: None,
            })
        }
        EditorAction::LoadScene { path } => {
            // `load_scene_from_file` needs `&mut World` while the registry
            // lives inside the world's resource storage; the scope holds the
            // registry out for the call's duration. The load is
            // failure-atomic for entities (moonfield-scene's commit
            // semantics), so a rejected document leaves nothing behind.
            let roots = world
                .try_resource_scope(|world, registry: &mut SceneRegistry| {
                    moonfield_scene::load_scene_from_file(world, registry, path)
                })
                .ok_or(ActionError::MissingSceneRegistry)??;
            Ok(ActionOutcome {
                message: format!("Loaded {} ({} roots)", path.display(), roots.len()),
                entity: roots.first().copied(),
                undo: (!roots.is_empty()).then_some(EditorUndo::DeleteEntities(roots)),
            })
        }
        EditorAction::SaveScene { path } => {
            world
                .try_resource_scope(|world, registry: &mut SceneRegistry| {
                    moonfield_scene::save_scene_to_file(world, registry, path)
                })
                .ok_or(ActionError::MissingSceneRegistry)??;
            Ok(ActionOutcome {
                message: format!("Saved {}", path.display()),
                entity: None,
                undo: None,
            })
        }
        EditorAction::SetTransform { entity, transform } => {
            let before = world.get_component::<Transform>(*entity).copied().ok_or(
                if world.contains(*entity) {
                    ActionError::MissingTransform(*entity)
                } else {
                    ActionError::MissingEntity(*entity)
                },
            )?;
            *world
                .get_component_mut::<Transform>(*entity)
                .expect("checked above") = *transform;
            Ok(ActionOutcome {
                message: format!("Moved {entity:?}"),
                entity: Some(*entity),
                undo: Some(EditorUndo::SetTransform {
                    entity: *entity,
                    transform: before,
                }),
            })
        }
        EditorAction::Batch(actions) => {
            validate_batch(actions)?;
            let mut undos = Vec::new();
            let mut entity = None;
            for action in actions {
                match execute(world, action) {
                    Ok(outcome) => {
                        undos.extend(outcome.undo);
                        entity = outcome.entity.or(entity);
                    }
                    Err(error) => {
                        let errors: Vec<ActionError> = undos
                            .iter()
                            .rev()
                            .filter_map(|undo: &EditorUndo| undo.apply(world).err())
                            .collect();
                        if !errors.is_empty() {
                            return Err(ActionError::Rollback {
                                cause: Box::new(error),
                                errors,
                            });
                        }
                        return Err(error);
                    }
                }
            }
            Ok(ActionOutcome {
                message: format!("Batched {} actions", actions.len()),
                entity,
                undo: (!undos.is_empty())
                    .then(|| EditorUndo::Batch(undos.into_iter().rev().collect())),
            })
        }
    }
}

fn validate_batch(actions: &[EditorAction]) -> Result<(), ActionError> {
    for action in actions {
        match action {
            EditorAction::DeleteEntity { .. } => {
                return Err(ActionError::IrreversibleBatch("DeleteEntity"));
            }
            EditorAction::SaveScene { .. } => {
                return Err(ActionError::IrreversibleBatch("SaveScene"));
            }
            EditorAction::Batch(nested) => validate_batch(nested)?,
            _ => {}
        }
    }
    Ok(())
}

/// One performed action, as both directions.
struct HistoryEntry {
    undo: EditorUndo,
    redo: EditorAction,
}

/// The undo/redo record of performed actions. A world resource, created on
/// first use; [`perform`], [`record`], [`undo`], and [`redo`] are the whole
/// interface.
#[derive(Default)]
pub struct EditorHistory {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

/// Execute `action` and record its inverse in the world's [`EditorHistory`]
/// Reversible edits and explicit deletion clear redo; saving leaves history
/// unchanged.
pub fn perform(world: &mut World, action: EditorAction) -> Result<ActionOutcome, ActionError> {
    let outcome = execute(world, &action)?;
    if matches!(action, EditorAction::DeleteEntity { .. })
        && let Some(mut history) = world.get_resource_mut::<EditorHistory>()
    {
        history.redo.clear();
    }
    if let Some(undo) = outcome.undo.clone() {
        world.init_resource::<EditorHistory>();
        world
            .get_resource_mut::<EditorHistory>()
            .expect("EditorHistory was just ensured")
            .push(HistoryEntry { undo, redo: action });
    }
    Ok(outcome)
}

/// Record an already-applied reversible edit — a continuous gesture that
/// applied its own increments (e.g. a gizmo drag), turned into one undo
/// unit. `done` is the gesture's end state, `undo` restores the start.
pub fn record(world: &mut World, done: EditorAction, undo: EditorUndo) {
    world.init_resource::<EditorHistory>();
    world
        .get_resource_mut::<EditorHistory>()
        .expect("EditorHistory was just ensured")
        .push(HistoryEntry { undo, redo: done });
}

impl EditorHistory {
    fn push(&mut self, entry: HistoryEntry) {
        self.undo.push(entry);
        self.redo.clear();
    }
}

/// Undo the most recent recorded action. `None` when there is nothing to
/// undo; `Some(Err(_))` when the inverse no longer applies (e.g. its entity
/// is gone).
pub fn undo(world: &mut World) -> Option<Result<ActionOutcome, ActionError>> {
    world
        .try_resource_scope(|world, history: &mut EditorHistory| {
            history.undo.pop().map(|entry| {
                let result = entry.undo.apply(world);
                if result.is_ok() {
                    history.redo.push(entry);
                } else {
                    history.undo.push(entry);
                }
                result
            })
        })
        .flatten()
}

/// Redo the most recently undone action.
pub fn redo(world: &mut World) -> Option<Result<ActionOutcome, ActionError>> {
    world
        .try_resource_scope(|world, history: &mut EditorHistory| {
            history.redo.pop().map(|mut entry| {
                let result = execute(world, &entry.redo);
                match &result {
                    Ok(outcome) => {
                        if let Some(undo) = &outcome.undo {
                            entry.undo = undo.clone();
                            history.undo.push(entry);
                        }
                    }
                    Err(_) => history.redo.push(entry),
                }
                result
            })
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_ecs::Name;

    /// A minimal but valid one-triangle GLB (non-indexed).
    fn test_mesh_glb() -> Vec<u8> {
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                      "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}}
                ],
                "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}}}]}}]
            }}"#,
            bin.len()
        );
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    fn write_temp(tag: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moonfield-action-test-{}-{tag}",
            std::process::id()
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn test_import_asset_is_undoable() {
        let path = write_temp("import.glb", &test_mesh_glb());
        let mut world = World::new();
        let outcome = perform(&mut world, EditorAction::ImportAsset { path: path.clone() })
            .expect("import succeeds");
        let entity = outcome.entity.expect("imported entity");
        assert!(world.contains(entity));

        undo(&mut world).expect("undoable").expect("undo succeeds");
        assert!(!world.contains(entity));

        redo(&mut world).expect("redoable").expect("redo succeeds");
        assert_eq!(world.iter_entities().count(), 1);
        for _ in 0..2 {
            undo(&mut world).unwrap().unwrap();
            assert_eq!(world.iter_entities().count(), 0);
            redo(&mut world).unwrap().unwrap();
            assert_eq!(world.iter_entities().count(), 1);
        }

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_failed_import_records_no_undo() {
        let mut world = World::new();
        let result = perform(
            &mut world,
            EditorAction::ImportAsset {
                path: PathBuf::from("does/not/exist.glb"),
            },
        );
        assert!(result.is_err());
        assert!(undo(&mut world).is_none());
    }

    #[test]
    fn test_set_transform_is_reversible() {
        let mut world = World::new();
        let entity = world.spawn((Name::new("moved"), Transform::IDENTITY));
        let moved = Transform::from_xyz(1.0, 2.0, 3.0);

        perform(
            &mut world,
            EditorAction::SetTransform {
                entity,
                transform: moved,
            },
        )
        .unwrap();
        assert_eq!(world.get_component::<Transform>(entity), Some(&moved));

        undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world.get_component::<Transform>(entity),
            Some(&Transform::IDENTITY)
        );
        redo(&mut world).unwrap().unwrap();
        assert_eq!(world.get_component::<Transform>(entity), Some(&moved));
    }

    #[test]
    fn test_set_transform_validates_entity() {
        let mut world = World::new();
        let entity = world.spawn_empty();
        let missing = world.spawn((Transform::IDENTITY,));
        world.despawn(missing).unwrap();

        // No Transform component.
        assert!(matches!(
            execute(
                &mut world,
                &EditorAction::SetTransform {
                    entity,
                    transform: Transform::IDENTITY,
                },
            ),
            Err(ActionError::MissingTransform(_))
        ));
        // Entity gone entirely.
        assert!(matches!(
            execute(
                &mut world,
                &EditorAction::SetTransform {
                    entity: missing,
                    transform: Transform::IDENTITY,
                },
            ),
            Err(ActionError::MissingEntity(_))
        ));
    }

    #[test]
    fn test_failed_history_operation_can_be_retried() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        let moved = Transform::from_xyz(1.0, 0.0, 0.0);
        perform(
            &mut world,
            EditorAction::SetTransform {
                entity,
                transform: moved,
            },
        )
        .unwrap();
        world.remove_component::<Transform>(entity).unwrap();
        assert!(undo(&mut world).unwrap().is_err());
        world.insert_component(entity, moved).unwrap();
        undo(&mut world).unwrap().unwrap();
        assert_eq!(
            world.get_component::<Transform>(entity),
            Some(&Transform::IDENTITY)
        );
        world.remove_component::<Transform>(entity).unwrap();
        assert!(redo(&mut world).unwrap().is_err());
        world.insert_component(entity, Transform::IDENTITY).unwrap();
        redo(&mut world).unwrap().unwrap();
        assert_eq!(world.get_component::<Transform>(entity), Some(&moved));
    }

    #[test]
    fn test_irreversible_edit_clears_redo() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        perform(
            &mut world,
            EditorAction::SetTransform {
                entity,
                transform: Transform::from_xyz(1.0, 0.0, 0.0),
            },
        )
        .unwrap();
        undo(&mut world).unwrap().unwrap();
        perform(&mut world, EditorAction::DeleteEntity { entity }).unwrap();
        assert!(redo(&mut world).is_none());
    }

    #[test]
    fn test_record_turns_gesture_into_undo_unit() {
        // A gizmo drag writes its increments directly, then records the
        // whole gesture as one unit — undo must restore the pre-drag
        // transform, redo the end state.
        let mut world = World::new();
        let entity = world.spawn((Name::new("dragged"), Transform::IDENTITY));
        let moved = Transform::from_xyz(4.0, 0.0, 0.0);
        *world.get_component_mut::<Transform>(entity).unwrap() = moved;

        record(
            &mut world,
            EditorAction::SetTransform {
                entity,
                transform: moved,
            },
            EditorUndo::SetTransform {
                entity,
                transform: Transform::IDENTITY,
            },
        );

        undo(&mut world).expect("recorded").expect("undo succeeds");
        assert_eq!(
            world.get_component::<Transform>(entity),
            Some(&Transform::IDENTITY)
        );
        redo(&mut world).expect("redoable").expect("redo succeeds");
        assert_eq!(world.get_component::<Transform>(entity), Some(&moved));
    }

    #[test]
    fn test_batch_rolls_back_on_failure() {
        let mut world = World::new();
        let entity = world.spawn((Name::new("moved"), Transform::IDENTITY));
        let ghost = world.spawn_empty();
        world.despawn(ghost).unwrap();

        let result = perform(
            &mut world,
            EditorAction::Batch(vec![
                EditorAction::SetTransform {
                    entity,
                    transform: Transform::from_xyz(5.0, 0.0, 0.0),
                },
                EditorAction::SetTransform {
                    entity: ghost,
                    transform: Transform::IDENTITY,
                },
            ]),
        );
        assert!(result.is_err());
        // The completed transform edit was rolled back.
        assert_eq!(
            world.get_component::<Transform>(entity),
            Some(&Transform::IDENTITY)
        );
        // The batch recorded no undo entry.
        assert!(undo(&mut world).is_none());
    }

    #[test]
    fn test_batch_rejects_irreversible_actions_before_mutation() {
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        let actions = EditorAction::Batch(vec![
            EditorAction::SetTransform {
                entity,
                transform: Transform::from_xyz(3.0, 0.0, 0.0),
            },
            EditorAction::Batch(vec![EditorAction::DeleteEntity { entity }]),
        ]);
        assert!(execute(&mut world, &actions).is_err());
        assert_eq!(
            world.get_component::<Transform>(entity),
            Some(&Transform::IDENTITY)
        );
        let path = std::env::temp_dir().join("moonfield-batch-rejected-save.gltf");
        world.insert_resource(crate::scene_io::editor_scene_registry());
        assert!(
            execute(
                &mut world,
                &EditorAction::Batch(vec![EditorAction::SaveScene { path: path.clone() }])
            )
            .is_err()
        );
        assert!(!path.exists());
    }

    #[test]
    fn test_batch_reports_failed_compensation() {
        struct Victim(Entity);
        struct BreakTransform;
        impl moonfield_scene::SceneTemplate for BreakTransform {
            fn insert_into_world(
                &self,
                world: &mut World,
                _: Entity,
            ) -> Result<(), moonfield_scene::SceneError> {
                let victim = world.get_resource::<Victim>().unwrap().0;
                world.remove_component::<Transform>(victim).unwrap();
                Err(moonfield_scene::SceneError::Invalid(
                    "template rejected".into(),
                ))
            }
        }
        let mut world = World::new();
        let entity = world.spawn((Transform::IDENTITY,));
        world.insert_resource(Victim(entity));
        let mut registry = SceneRegistry::new();
        registry.register_custom("break", |_, _| None, |_| Ok(Box::new(BreakTransform)));
        world.insert_resource(registry);
        let path = write_temp("rollback.gltf", br#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"extras":{"components":{"break":null}}}]}"#);
        let error = execute(
            &mut world,
            &EditorAction::Batch(vec![
                EditorAction::SetTransform {
                    entity,
                    transform: Transform::from_xyz(2.0, 0.0, 0.0),
                },
                EditorAction::LoadScene { path: path.clone() },
            ]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("rollback failed"), "{error}");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_batch_imports_undo_and_redo_as_one_unit() {
        let path = write_temp("batch-import.glb", &test_mesh_glb());
        let mut world = World::new();
        perform(
            &mut world,
            EditorAction::Batch(vec![
                EditorAction::ImportAsset { path: path.clone() },
                EditorAction::ImportAsset { path: path.clone() },
            ]),
        )
        .unwrap();
        for _ in 0..2 {
            assert_eq!(world.iter_entities().count(), 2);
            undo(&mut world).unwrap().unwrap();
            assert_eq!(world.iter_entities().count(), 0);
            redo(&mut world).unwrap().unwrap();
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_scene_load_undo_despawns_roots() {
        let mut saved_world = World::new();
        saved_world.register_hierarchy();
        saved_world.spawn((Name::new("root"), Transform::from_xyz(1.0, 0.0, 0.0)));
        let path = std::env::temp_dir().join(format!(
            "moonfield-action-test-{}-scene.gltf",
            std::process::id()
        ));
        moonfield_scene::save_scene_to_file(
            &saved_world,
            &crate::scene_io::editor_scene_registry(),
            &path,
        )
        .unwrap();

        let mut world = World::new();
        world.register_hierarchy();
        world.insert_resource(crate::scene_io::editor_asset_server());
        world.insert_resource(crate::scene_io::editor_scene_registry());

        let outcome = perform(&mut world, EditorAction::LoadScene { path: path.clone() })
            .expect("load succeeds");
        assert!(outcome.entity.is_some());
        assert_eq!(world.iter_entities().count(), 1);

        undo(&mut world).unwrap().unwrap();
        assert_eq!(world.iter_entities().count(), 0);
        for _ in 0..2 {
            redo(&mut world).unwrap().unwrap();
            assert_eq!(world.iter_entities().count(), 1);
            undo(&mut world).unwrap().unwrap();
            assert_eq!(world.iter_entities().count(), 0);
        }

        std::fs::remove_file(&path).ok();
    }
}

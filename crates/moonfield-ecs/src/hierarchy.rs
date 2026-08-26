//! The built-in parent/child hierarchy: [`ChildOf`] / [`Children`] on top of
//! the generic [`Relationship`](crate::Relationship) mechanism, plus the
//! `Transform` → `GlobalTransform` propagation systems.
//!
//! Register once per world via [`World::register_hierarchy`] (the app-side
//! `HierarchyPlugin` does this and schedules the propagation systems).
//!
//! Semantics (Bevy 0.20-aligned):
//! - inserting/replacing [`ChildOf`] links the child into the parent's
//!   [`Children`] (auto-created if missing); replacing first unlinks from the
//!   old parent;
//! - removing [`ChildOf`] or despawning the child unlinks it; an emptied
//!   [`Children`] component is removed;
//! - despawning a parent **despawns its children recursively** (`linked_spawn`);
//! - inserting [`ChildOf`] that would create a cycle **panics** (checked by
//!   walking the ancestor chain — Bevy does not prevent cycles, we do).

use moonfield_math::{Affine3A, GlobalTransform, Transform};

use crate::relationship::relationship_on_insert;
use crate::{Commands, Entity, Query, Relationship, RelationshipTarget, World};

/// The child → parent relationship: `ChildOf(parent)` is stored on the child.
///
/// This is Bevy 0.20's naming (`ChildOf`, replacing the older `Parent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildOf(pub Entity);

impl ChildOf {
    /// The parent entity.
    pub fn parent(&self) -> Entity {
        self.0
    }
}

impl Relationship for ChildOf {
    type Target = Children;

    fn target(&self) -> Entity {
        self.0
    }

    fn from(entity: Entity) -> Self {
        Self(entity)
    }
}

/// The parent → children relationship target: every entity with at least one
/// child. Auto-created on first link and auto-removed when it empties.
///
/// Dereferences to `[Entity]` in insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Children(Vec<Entity>);

impl RelationshipTarget for Children {
    type Relationship = ChildOf;

    /// Despawning a parent despawns its children, recursively.
    const LINKED_SPAWN: bool = true;

    fn entities(&self) -> &[Entity] {
        &self.0
    }

    fn add_entity(&mut self, entity: Entity) {
        if !self.0.contains(&entity) {
            self.0.push(entity);
        }
    }

    fn remove_entity(&mut self, entity: Entity) {
        if let Some(index) = self.0.iter().position(|&e| e == entity) {
            self.0.swap_remove(index);
        }
    }
}

impl std::ops::Deref for Children {
    type Target = [Entity];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Reject a `child → parent` link that would close a cycle, by walking the
/// ancestor chain from `parent`; `child` must not appear in it.
fn assert_acyclic(world: &World, child: Entity, parent: Entity) {
    let mut current = parent;
    loop {
        if current == child {
            panic!(
                "hierarchy cycle: making {parent:?} the parent of {child:?} would create a cycle"
            );
        }
        match world.get_component::<ChildOf>(current) {
            Some(child_of) => current = child_of.parent(),
            None => break,
        }
    }
}

impl World {
    /// Register the [`ChildOf`] / [`Children`] relationship: the generic
    /// link/unlink machinery, linked-spawn despawn, and cycle prevention.
    ///
    /// Call once per world before using hierarchy components (the app-side
    /// `HierarchyPlugin` does this).
    pub fn register_hierarchy(&mut self) {
        self.register_relationship::<ChildOf>();
        // Override the insert hook to additionally reject cycles before linking.
        self.register_component_hooks::<ChildOf>()
            .on_insert(|world, entity| {
                let parent = world.get_component::<ChildOf>(entity).unwrap().parent();
                assert_acyclic(world, entity, parent);
                relationship_on_insert::<ChildOf>(world, entity);
            });
    }
}

// ---------------------------------------------------------------------
// Transform propagation
// ---------------------------------------------------------------------

/// Inserts [`GlobalTransform::IDENTITY`] on every entity that has a
/// [`Transform`] but no [`GlobalTransform`] yet.
///
/// Register before [`propagate_transforms`] in the same schedule; the queued
/// inserts apply between the two systems, so new entities are propagated in
/// the same run.
pub fn ensure_global_transforms(
    transforms: Query<&Transform>,
    globals: Query<&GlobalTransform>,
    commands: Commands,
) {
    for (entity, _) in transforms.iter() {
        if globals.get(entity).is_none() {
            commands.entity(entity).insert((GlobalTransform::IDENTITY,));
        }
    }
}

/// Propagates [`Transform`]s down the hierarchy into [`GlobalTransform`]s:
/// roots (entities with a [`Transform`] and no [`ChildOf`]) take their local
/// affine as global; every child composes `parent_global * local`,
/// recursively.
///
/// Entities with a [`ChildOf`] link whose ancestor chain has no [`Transform`]
/// root are not reached (their global stays stale) — same coverage as Bevy's
/// propagation query. Children without their own [`Transform`] are skipped.
pub fn propagate_transforms(
    transforms: Query<&Transform>,
    childofs: Query<&ChildOf>,
    children: Query<&Children>,
    mut globals: Query<&mut GlobalTransform>,
) {
    for (entity, local) in transforms.iter() {
        if childofs.get(entity).is_some() {
            // Not a root: reached through its ancestor's recursion.
            continue;
        }
        let affine = local.compute_affine();
        if let Some(mut global) = globals.get(entity) {
            global.set_affine(affine);
        }
        if let Some(kids) = children.get(entity) {
            propagate_children(&kids, affine, &transforms, &children, &mut globals);
        }
    }
}

fn propagate_children(
    kids: &Children,
    parent: Affine3A,
    transforms: &Query<&Transform>,
    children: &Query<&Children>,
    globals: &mut Query<&mut GlobalTransform>,
) {
    for &child in kids.entities() {
        let Some(local) = transforms.get(child) else {
            continue;
        };
        let affine = parent * local.compute_affine();
        drop(local);
        if let Some(mut global) = globals.get(child) {
            global.set_affine(affine);
        }
        if let Some(grandkids) = children.get(child) {
            propagate_children(&grandkids, affine, transforms, children, globals);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoSystemConfigs, Schedule};
    use moonfield_math::{Quat, Vec3};

    fn approx(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-5, "{a} != {b}");
    }

    fn hierarchy_world() -> World {
        let mut world = World::new();
        world.register_hierarchy();
        world
    }

    fn propagation_schedule() -> Schedule {
        let mut schedule = Schedule::new();
        schedule.add_systems((
            ensure_global_transforms,
            propagate_transforms.after(&ensure_global_transforms),
        ));
        schedule
    }

    #[test]
    fn test_insert_childof_links_parent_children() {
        let mut world = hierarchy_world();
        let parent = world.spawn(());
        let child = world.spawn((ChildOf(parent),));

        let children = world.get_component::<Children>(parent).unwrap();
        assert_eq!(children.entities(), &[child]);
        assert_eq!(
            world.get_component::<ChildOf>(child).unwrap().parent(),
            parent
        );
    }

    #[test]
    fn test_remove_childof_unlinks_and_drops_empty_children() {
        let mut world = hierarchy_world();
        let parent = world.spawn(());
        let child = world.spawn((ChildOf(parent),));

        world.remove_component::<ChildOf>(child);
        assert!(world.get_component::<Children>(parent).is_none());
    }

    #[test]
    fn test_reparent_moves_the_link() {
        let mut world = hierarchy_world();
        let p1 = world.spawn(());
        let p2 = world.spawn(());
        let child = world.spawn((ChildOf(p1),));

        world.insert_component(child, ChildOf(p2));
        assert!(world.get_component::<Children>(p1).is_none());
        assert_eq!(
            world.get_component::<Children>(p2).unwrap().entities(),
            &[child]
        );
    }

    #[test]
    fn test_despawn_child_unlinks_from_parent() {
        let mut world = hierarchy_world();
        let parent = world.spawn(());
        let child = world.spawn((ChildOf(parent),));
        let sibling = world.spawn((ChildOf(parent),));

        world.despawn(child).unwrap();
        assert_eq!(
            world.get_component::<Children>(parent).unwrap().entities(),
            &[sibling]
        );
    }

    #[test]
    fn test_despawn_parent_despawns_children_recursively() {
        let mut world = hierarchy_world();
        let root = world.spawn(());
        let child = world.spawn((ChildOf(root),));
        let grandchild = world.spawn((ChildOf(child),));

        world.despawn(root).unwrap();
        assert!(!world.contains(child));
        assert!(!world.contains(grandchild));
    }

    #[test]
    fn test_manual_children_removal_unlinks_but_keeps_children_alive() {
        let mut world = hierarchy_world();
        let parent = world.spawn(());
        let child = world.spawn((ChildOf(parent),));

        // Removing the target component manually unlinks (Bevy semantics);
        // only despawning the parent despawns children.
        world.remove_component::<Children>(parent);
        assert!(world.contains(child));
        assert!(world.get_component::<ChildOf>(child).is_none());
    }

    #[test]
    #[should_panic(expected = "hierarchy cycle")]
    fn test_cycle_is_rejected() {
        let mut world = hierarchy_world();
        let a = world.spawn(());
        let b = world.spawn((ChildOf(a),));
        world.insert_component(a, ChildOf(b));
    }

    #[test]
    #[should_panic(expected = "hierarchy cycle")]
    fn test_self_parenting_is_rejected() {
        let mut world = hierarchy_world();
        let a = world.spawn(());
        world.insert_component(a, ChildOf(a));
    }

    #[test]
    fn test_commands_deferred_spawn_maintains_children() {
        let mut world = hierarchy_world();
        let (parent, child) = {
            let commands = Commands::new(&world);
            let parent = commands
                .spawn((Transform::from_xyz(1.0, 0.0, 0.0),))
                .entity();
            let child = commands
                .spawn((Transform::from_xyz(0.0, 1.0, 0.0), ChildOf(parent)))
                .entity();
            (parent, child)
        };
        world.apply_commands();

        assert_eq!(
            world.get_component::<Children>(parent).unwrap().entities(),
            &[child]
        );
    }

    #[test]
    fn test_ensure_global_transforms_backfills_missing() {
        let mut world = hierarchy_world();
        let e = world.spawn((Transform::from_xyz(1.0, 2.0, 3.0),));

        let mut schedule = propagation_schedule();
        schedule.run(&mut world);

        let global = world.get_component::<GlobalTransform>(e).unwrap();
        approx(global.translation(), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn test_propagation_composes_nested_trs() {
        let mut world = hierarchy_world();
        let root = world.spawn((Transform::from_xyz(1.0, 0.0, 0.0),));
        let child = world.spawn((
            Transform {
                translation: Vec3::new(0.0, 1.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::splat(2.0),
            },
            ChildOf(root),
        ));
        let grandchild = world.spawn((
            Transform {
                translation: Vec3::new(1.0, 0.0, 0.0),
                rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                scale: Vec3::ONE,
            },
            ChildOf(child),
        ));

        let mut schedule = propagation_schedule();
        schedule.run(&mut world);

        // Root: global == local.
        let root_global = world.get_component::<GlobalTransform>(root).unwrap();
        approx(root_global.translation(), Vec3::new(1.0, 0.0, 0.0));

        // Child: translated by the root, scaled ×2.
        let child_global = world.get_component::<GlobalTransform>(child).unwrap();
        approx(child_global.translation(), Vec3::new(1.0, 1.0, 0.0));
        approx(
            child_global.affine().transform_vector3(Vec3::X),
            Vec3::new(2.0, 0.0, 0.0),
        );

        // Grandchild: its local +X is scaled ×2 by the child, then rotated
        // 90° about Z by its own rotation → +Y ×2; its origin lands at
        // (1,1,0) + (2,0,0).
        let grandchild_global = world.get_component::<GlobalTransform>(grandchild).unwrap();
        approx(grandchild_global.translation(), Vec3::new(3.0, 1.0, 0.0));
        approx(
            grandchild_global.affine().transform_vector3(Vec3::X),
            Vec3::new(0.0, 2.0, 0.0),
        );
    }

    #[test]
    fn test_propagation_via_commands_spawned_hierarchy() {
        let mut world = hierarchy_world();
        {
            let commands = Commands::new(&world);
            let root = commands
                .spawn((Transform::from_xyz(5.0, 0.0, 0.0),))
                .entity();
            commands.spawn((Transform::from_xyz(0.0, 5.0, 0.0), ChildOf(root)));
        }
        // Commands queued outside a schedule run apply lazily (at the latest
        // after the next run's first system); flush explicitly so the
        // hierarchy exists before propagation.
        world.apply_commands();
        let mut schedule = propagation_schedule();
        schedule.run(&mut world);

        // The child (spawned and linked via deferred commands, globals
        // backfilled the same run) ends up at the sum of the translations.
        let mut found = false;
        for (_, (childof, global)) in world.query::<(&ChildOf, &GlobalTransform)>() {
            approx(global.translation(), Vec3::new(5.0, 5.0, 0.0));
            let _ = childof;
            found = true;
        }
        assert!(found);
    }

    #[test]
    fn test_query_get_guards_release_borrows() {
        use crate::SystemParam;

        let mut world = World::new();
        let e = world.spawn((Transform::from_xyz(1.0, 2.0, 3.0),));

        // Two sequential gets on the same column must not conflict.
        {
            let transforms = <Query<&Transform> as SystemParam>::fetch(&world, &mut ());
            let t = transforms.get(e).unwrap();
            approx(t.translation, Vec3::new(1.0, 2.0, 3.0));
        }
        // A mutable get after the shared guard dropped.
        {
            let transforms = <Query<&mut Transform> as SystemParam>::fetch(&world, &mut ());
            let mut t = transforms.get(e).unwrap();
            t.translation.x = 9.0;
        }
        assert_eq!(
            world.get_component::<Transform>(e).unwrap().translation,
            Vec3::new(9.0, 2.0, 3.0)
        );
    }
}

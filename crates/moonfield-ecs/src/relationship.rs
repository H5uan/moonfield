//! Generic entity relationships, ported from Bevy 0.20's
//! `bevy_ecs::relationship` (architecture-level; the derive becomes
//! hand-written trait impls here).
//!
//! A [`Relationship`] is a component on the *source* entity pointing at a
//! *target* entity (e.g. `ChildOf(parent)`). The matching
//! [`RelationshipTarget`] is the component on the target holding the
//! collection of sources (e.g. `Children`). The two sides are kept in sync by
//! component lifecycle hooks, registered once per relationship type via
//! [`World::register_relationship`]:
//!
//! - inserting / replacing a relationship links the source into the target's
//!   collection (creating the target component if missing);
//! - discarding a relationship (remove/replace/despawn of the source)
//!   unlinks it, dropping the target component once it is empty;
//! - discarding a relationship target (remove/despawn of the target entity)
//!   either unlinks every source (default) or despawns them
//!   ([`RelationshipTarget::LINKED_SPAWN`], Bevy's `linked_spawn`).
//!
//! Differences from Bevy, documented deliberately: hooks run immediately
//! (single-threaded, no command deferral inside hooks); an insert pointing at
//! the entity itself **panics** unless
//! [`Relationship::ALLOW_SELF_REFERENTIAL`] (Bevy warns and removes); an
//! insert whose target does not exist silently discards the relationship
//! component (Bevy warns and removes).

use std::any::type_name;

use crate::{Component, Entity, World};

/// A component on the *source* side of a relationship, pointing at a target
/// entity. See the module docs.
pub trait Relationship: Component {
    /// The component stored on target entities, holding all sources.
    type Target: RelationshipTarget<Relationship = Self>;

    /// If `true`, a relationship may point at its own entity.
    const ALLOW_SELF_REFERENTIAL: bool = false;

    /// The entity this relationship points to.
    fn target(&self) -> Entity;

    /// Construct a relationship pointing at `entity`.
    fn from(entity: Entity) -> Self;
}

/// A component on the *target* side of a relationship: the collection of
/// source entities. See the module docs.
pub trait RelationshipTarget: Component + Default {
    /// The relationship that populates this collection.
    type Relationship: Relationship<Target = Self>;

    /// When `true`, despawning (or removing this component from) the target
    /// entity despawns every source entity (Bevy's `linked_spawn`). When
    /// `false`, the sources only lose their relationship component.
    const LINKED_SPAWN: bool;

    /// The source entities, in insertion order.
    fn entities(&self) -> &[Entity];

    /// Add a source to the collection, deduplicated.
    #[doc(hidden)]
    fn add_entity(&mut self, entity: Entity);

    /// Remove a source from the collection.
    #[doc(hidden)]
    fn remove_entity(&mut self, entity: Entity);
}

/// `on_insert` hook maintaining a [`Relationship`] → [`RelationshipTarget`]
/// link: adds the source to the target's collection, creating the target
/// component if necessary.
pub(crate) fn relationship_on_insert<R: Relationship>(world: &mut World, entity: Entity) {
    let Some(target) = world.get_component::<R>(entity).map(|r| r.target()) else {
        return;
    };
    if !R::ALLOW_SELF_REFERENTIAL && target == entity {
        panic!(
            "relationship `{}` on {entity:?} points to itself",
            type_name::<R>()
        );
    }
    if !world.contains(target) {
        // The target does not exist: discard the invalid relationship (Bevy
        // warns and removes; we have no warning channel here).
        world.remove_component::<R>(entity);
        return;
    }
    if world.get_component::<R::Target>(target).is_none() {
        world.insert_component(target, R::Target::default());
    }
    if let Some(mut collection) = world.get_component_mut::<R::Target>(target) {
        collection.add_entity(entity);
    }
}

/// `on_discard` hook maintaining the link: removes the source from the
/// target's collection while the relationship value is still readable, and
/// drops the target component once the collection is empty.
pub(crate) fn relationship_on_discard<R: Relationship>(world: &mut World, entity: Entity) {
    let Some(target) = world.get_component::<R>(entity).map(|r| r.target()) else {
        return;
    };
    let Some(mut collection) = world.get_component_mut::<R::Target>(target) else {
        return;
    };
    collection.remove_entity(entity);
    let empty = collection.entities().is_empty();
    if empty {
        world.remove_component::<R::Target>(target);
    }
}

/// `on_discard` hook for the target side: when the target component is
/// removed (manually or via despawn), unlink every source — its relationship
/// component is removed. Runs while the collection is still readable.
pub(crate) fn relationship_target_on_discard<R: Relationship>(world: &mut World, entity: Entity) {
    let sources = world
        .get_component::<R::Target>(entity)
        .map(|t| t.entities().to_vec())
        .unwrap_or_default();
    for source in sources {
        world.remove_component::<R>(source);
    }
}

/// `on_despawn` hook for the target side: with
/// [`RelationshipTarget::LINKED_SPAWN`], despawning the target entity despawns
/// every source entity recursively. Runs before the discard hooks, while the
/// collection is still intact; each source's own discard hook unlinks it from
/// the collection as it goes down.
///
/// The full descendant closure is collected up front: while this hook runs,
/// the same hook on nested targets is suppressed (the recursion guard), so
/// nested sources cannot rely on it firing again.
pub(crate) fn relationship_target_on_despawn<R: Relationship>(world: &mut World, entity: Entity) {
    if !R::Target::LINKED_SPAWN {
        return;
    }
    let mut stack = world
        .get_component::<R::Target>(entity)
        .map(|t| t.entities().to_vec())
        .unwrap_or_default();
    let mut descendants = Vec::new();
    while let Some(source) = stack.pop() {
        descendants.push(source);
        if let Some(target) = world.get_component::<R::Target>(source) {
            stack.extend(target.entities().iter().copied());
        }
    }
    for source in descendants {
        // Sources deeper in the closure may already be gone (unlink hooks run
        // as parents despawn); ignore those.
        let _ = world.despawn(source);
    }
}

impl World {
    /// Register the lifecycle hooks that keep a [`Relationship`] /
    /// [`RelationshipTarget`] pair in sync. Call once per relationship type,
    /// before any entity uses it.
    pub fn register_relationship<R: Relationship>(&mut self) {
        self.register_component_hooks::<R>()
            .on_insert(relationship_on_insert::<R>)
            .on_discard(relationship_on_discard::<R>);
        self.register_component_hooks::<R::Target>()
            .on_discard(relationship_target_on_discard::<R>)
            .on_despawn(relationship_target_on_despawn::<R>);
    }
}

#[cfg(test)]
mod tests {
    use crate::{Entity, Relationship, RelationshipTarget, World};

    /// A non-linked relationship for exercising the generic machinery:
    /// despawning the target unlinks sources instead of despawning them.
    #[derive(Debug, PartialEq)]
    struct Likes(Entity);

    impl Relationship for Likes {
        type Target = LikedBy;

        fn target(&self) -> Entity {
            self.0
        }

        fn from(entity: Entity) -> Self {
            Self(entity)
        }
    }

    #[derive(Debug, Default, PartialEq)]
    struct LikedBy(Vec<Entity>);

    impl RelationshipTarget for LikedBy {
        type Relationship = Likes;

        const LINKED_SPAWN: bool = false;

        fn entities(&self) -> &[Entity] {
            &self.0
        }

        fn add_entity(&mut self, entity: Entity) {
            if !self.0.contains(&entity) {
                self.0.push(entity);
            }
        }

        fn remove_entity(&mut self, entity: Entity) {
            self.0.retain(|&e| e != entity);
        }
    }

    #[test]
    fn test_insert_links_into_target_collection() {
        let mut world = World::new();
        world.register_relationship::<Likes>();

        let target = world.spawn(());
        let fan = world.spawn((Likes(target),));

        let liked_by = world.get_component::<LikedBy>(target).unwrap();
        assert_eq!(liked_by.entities(), &[fan]);
    }

    #[test]
    fn test_replace_relinks_to_new_target() {
        let mut world = World::new();
        world.register_relationship::<Likes>();

        let a = world.spawn(());
        let b = world.spawn(());
        let fan = world.spawn((Likes(a),));
        world.insert_component(fan, Likes(b));

        assert!(world.get_component::<LikedBy>(a).is_none()); // emptied → removed
        assert_eq!(
            world.get_component::<LikedBy>(b).unwrap().entities(),
            &[fan]
        );
    }

    #[test]
    fn test_remove_unlinks_and_drops_empty_target() {
        let mut world = World::new();
        world.register_relationship::<Likes>();

        let target = world.spawn(());
        let fan = world.spawn((Likes(target),));
        world.remove_component::<Likes>(fan);

        assert!(world.get_component::<LikedBy>(target).is_none());
    }

    #[test]
    fn test_despawn_target_unlinks_sources_when_not_linked() {
        let mut world = World::new();
        world.register_relationship::<Likes>();

        let target = world.spawn(());
        let fan = world.spawn((Likes(target),));
        world.despawn(target).unwrap();

        // Non-linked: the source survives, minus its relationship component.
        assert!(world.contains(fan));
        assert!(world.get_component::<Likes>(fan).is_none());
    }

    #[test]
    fn test_despawn_source_unlinks_from_target() {
        let mut world = World::new();
        world.register_relationship::<Likes>();

        let target = world.spawn(());
        let fan = world.spawn((Likes(target),));
        world.despawn(fan).unwrap();

        // The emptied target component is removed.
        assert!(world.get_component::<LikedBy>(target).is_none());
    }

    #[test]
    fn test_relationship_to_nonexistent_target_is_discarded() {
        let mut world = World::new();
        world.register_relationship::<Likes>();

        let ghost = world.spawn(());
        world.despawn(ghost).unwrap();

        let fan = world.spawn(());
        world.insert_component(fan, Likes(ghost));
        assert!(world.get_component::<Likes>(fan).is_none());
    }

    #[test]
    #[should_panic(expected = "points to itself")]
    fn test_self_referential_relationship_panics() {
        let mut world = World::new();
        world.register_relationship::<Likes>();

        let e = world.spawn(());
        world.insert_component(e, Likes(e));
    }
}

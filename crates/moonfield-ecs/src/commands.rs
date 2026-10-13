//! Deferred structural world mutations, ported from Bevy's `Commands`.
//!
//! Systems receive a [`Commands`] param and queue entity/component mutations
//! instead of applying them immediately, so structural changes never happen
//! while queries are iterating. The queue lives on the [`World`] and is
//! drained by [`World::apply_commands`], which the schedule runner calls after
//! every system — a system's commands are therefore visible to every system
//! that runs after it in the same schedule run.

use crate::system::SystemParam;
use crate::{Bundle, Component, Entity, Resource, World};

/// A queued, deferred world mutation.
pub type Command = Box<dyn FnOnce(&mut World) + 'static>;

/// A queue of deferred world mutations, available as a system param.
///
/// Nothing is applied immediately: every method enqueues a [`Command`] onto
/// the world's command queue, applied by the schedule after the current
/// system finishes (see [`World::apply_commands`]).
///
/// Interior mutability (the queue is behind a `RefCell`) is what lets the
/// methods take `&self`.
pub struct Commands<'w> {
    world: &'w World,
}

impl<'w> Commands<'w> {
    pub(crate) fn new(world: &'w World) -> Self {
        Self { world }
    }

    /// Queue an arbitrary deferred mutation.
    pub fn queue(&self, command: impl FnOnce(&mut World) + 'static) {
        self.world.queue_command(Box::new(command));
    }

    /// Queue spawning an entity with the given bundle of components.
    ///
    /// The [`Entity`] is reserved immediately, so it can be referenced by
    /// later commands (or returned to the caller) before the spawn applies.
    pub fn spawn(&self, bundle: impl Bundle + 'static) -> EntityCommands<'w> {
        let entity = self.world.reserve_entity();
        self.queue(move |world| {
            world.spawn_at(entity, bundle);
        });
        EntityCommands {
            entity,
            world: self.world,
        }
    }

    /// Queue spawning an entity with no components.
    pub fn spawn_empty(&self) -> EntityCommands<'w> {
        self.spawn(())
    }

    /// Queue commands against an existing entity.
    pub fn entity(&self, entity: Entity) -> EntityCommands<'w> {
        EntityCommands {
            entity,
            world: self.world,
        }
    }

    /// Queue inserting (or replacing) a resource.
    pub fn insert_resource(&self, resource: impl Resource) {
        self.queue(move |world| world.insert_resource(resource));
    }
}

impl SystemParam for Commands<'_> {
    type State = ();
    type Item<'w, 's> = Commands<'w>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        Commands::new(world)
    }
}

/// Deferred mutations targeted at one entity, returned by [`Commands::spawn`]
/// and [`Commands::entity`].
pub struct EntityCommands<'w> {
    entity: Entity,
    world: &'w World,
}

impl EntityCommands<'_> {
    /// The entity these commands apply to.
    pub fn entity(&self) -> Entity {
        self.entity
    }

    /// Queue inserting (or replacing) a bundle of components.
    pub fn insert(&self, bundle: impl Bundle + 'static) -> &Self {
        let entity = self.entity;
        self.world.queue_command(Box::new(move |world| {
            world.insert_bundle(entity, bundle);
        }));
        self
    }

    /// Queue removing component `T`.
    pub fn remove<T: Component>(&self) -> &Self {
        let entity = self.entity;
        self.world.queue_command(Box::new(move |world| {
            world.remove_component::<T>(entity);
        }));
        self
    }

    /// Queue despawning the entity.
    pub fn despawn(&self) {
        let entity = self.entity;
        self.world.queue_command(Box::new(move |world| {
            let _ = world.despawn(entity);
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct A(u32);
    #[derive(Debug, PartialEq)]
    struct B(u32);

    fn apply(world: &mut World, f: impl FnOnce(Commands)) {
        f(Commands::new(world));
        world.apply_commands();
    }

    #[test]
    fn test_spawn_queues_until_apply() {
        let mut world = World::new();
        let entity = {
            let commands = Commands::new(&world);
            let entity = commands.spawn((A(1),)).entity();
            // Reserved immediately, but no components until applied.
            assert!(world.get_component::<A>(entity).is_none());
            entity
        };
        world.apply_commands();
        assert_eq!(world.get_component::<A>(entity), Some(&A(1)));
    }

    #[test]
    fn test_insert_remove_despawn() {
        let mut world = World::new();
        let entity = world.spawn((A(1),));

        apply(&mut world, |c| {
            c.entity(entity).insert((B(2),));
        });
        assert_eq!(world.get_component::<B>(entity), Some(&B(2)));
        assert_eq!(world.get_component::<A>(entity), Some(&A(1)));

        apply(&mut world, |c| {
            c.entity(entity).remove::<A>();
        });
        assert!(world.get_component::<A>(entity).is_none());
        assert_eq!(world.get_component::<B>(entity), Some(&B(2)));

        apply(&mut world, |c| {
            c.entity(entity).despawn();
        });
        assert!(!world.contains(entity));
    }

    #[test]
    fn test_commands_queued_by_commands_apply_in_same_pass() {
        let mut world = World::new();
        apply(&mut world, |c| {
            c.queue(|world| {
                Commands::new(world).spawn((A(7),));
            });
        });
        assert_eq!(world.query::<&A>().count(), 1);
    }

    #[test]
    fn test_insert_resource_via_commands() {
        let mut world = World::new();
        apply(&mut world, |c| c.insert_resource(A(3)));
        assert_eq!(world.get_resource::<A>().unwrap().0, 3);
    }
}

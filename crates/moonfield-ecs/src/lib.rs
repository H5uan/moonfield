use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;

mod commands;
mod component;
mod entity;
mod query;
mod resource;
mod system;
mod world;

pub use commands::{CommandQueue, Commands};
pub use component::{Component, ComponentStorage};
pub use entity::Entity;
pub use query::Query;
pub use resource::Resource;
pub use system::{IntoSystem, System};
pub use world::World;

/// Common ECS imports.
pub mod prelude {
    pub use crate::{Commands, Component, Entity, IntoSystem, Query, Resource, System, World};
}

/// Type-erased resource storage.
#[derive(Default)]
pub(crate) struct Resources {
    data: HashMap<TypeId, RefCell<Box<dyn Any>>>,
}

impl Resources {
    pub fn insert<R: Resource>(&mut self, res: R) {
        self.data
            .insert(TypeId::of::<R>(), RefCell::new(Box::new(res)));
    }

    pub fn contains<R: Resource>(&self) -> bool {
        self.data.contains_key(&TypeId::of::<R>())
    }

    pub fn get<R: Resource>(&self) -> Option<Ref<'_, R>> {
        let cell = self.data.get(&TypeId::of::<R>())?;
        // If already mutably borrowed, this will panic at runtime — acceptable for a minimal ECS.
        Some(Ref::map(cell.borrow(), |any| {
            any.downcast_ref::<R>().unwrap()
        }))
    }

    pub fn get_mut<R: Resource>(&self) -> Option<RefMut<'_, R>> {
        let cell = self.data.get(&TypeId::of::<R>())?;
        Some(RefMut::map(cell.borrow_mut(), |any| {
            any.downcast_mut::<R>().unwrap()
        }))
    }

    pub fn remove<R: Resource>(&mut self) -> Option<R> {
        self.data
            .remove(&TypeId::of::<R>())
            .map(|cell| *cell.into_inner().downcast::<R>().unwrap())
    }
}

/// World-local change queue used by [`Commands`].
#[derive(Default)]
pub(crate) struct EntityChanges {
    pub to_spawn: Vec<Vec<Box<dyn FnOnce(Entity, &mut World)>>>,
    pub to_despawn: Vec<Entity>,
}

impl EntityChanges {
    pub fn apply(&mut self, world: &mut World) {
        for bundle_fns in self.to_spawn.drain(..) {
            let e = world.spawn_empty();
            for f in bundle_fns {
                f(e, world);
            }
        }
        for e in self.to_despawn.drain(..) {
            world.despawn(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Velocity {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, PartialEq, Default)]
    struct FrameCounter(u32);

    #[test]
    fn spawn_entity_and_query() {
        let mut world = World::new();
        world.spawn((Position { x: 1.0, y: 2.0 },));
        world.spawn2(Position { x: 3.0, y: 4.0 }, Velocity { x: 0.5, y: 0.5 });

        let positions: Vec<_> = world.query::<&Position>().map(|p| p.clone()).collect();
        assert_eq!(
            positions,
            vec![Position { x: 1.0, y: 2.0 }, Position { x: 3.0, y: 4.0 }]
        );
    }

    #[test]
    fn query_mutable() {
        let mut world = World::new();
        world.spawn2(Position { x: 1.0, y: 2.0 }, Velocity { x: 1.0, y: 0.0 });

        for (mut pos, vel) in world.query_mut::<(&mut Position, &Velocity)>() {
            pos.x += vel.x;
            pos.y += vel.y;
        }

        let pos = world.query::<&Position>().next().unwrap();
        assert_eq!(pos.x, 2.0);
        assert_eq!(pos.y, 2.0);
    }

    #[test]
    fn resources_roundtrip() {
        let mut world = World::new();
        world.insert_resource(FrameCounter(7));
        assert_eq!(world.get_resource::<FrameCounter>().unwrap().0, 7);
        world.get_resource_mut::<FrameCounter>().unwrap().0 = 42;
        assert_eq!(world.get_resource::<FrameCounter>().unwrap().0, 42);
    }

    #[test]
    fn commands_spawn_and_despawn() {
        let mut world = World::new();

        // spawn via command
        {
            let mut cmds = world.commands();
            cmds.spawn((Position { x: 10.0, y: 20.0 },));
        }
        world.apply_commands();

        let pos: Vec<_> = world.query::<&Position>().map(|p| p.clone()).collect();
        assert_eq!(pos, vec![Position { x: 10.0, y: 20.0 }]);

        // despawn via command
        let entity = world.entities().alive_entities().next().unwrap();
        {
            let mut cmds = world.commands();
            cmds.despawn(entity);
        }
        world.apply_commands();

        let count = world.query::<&Position>().count();
        assert_eq!(count, 0);
    }

    #[test]
    fn system_runs_on_world() {
        fn update_positions(world: &mut World) {
            for (mut pos, vel) in world.query_mut::<(&mut Position, &Velocity)>() {
                pos.x += vel.x;
                pos.y += vel.y;
            }
        }

        let mut world = World::new();
        world.spawn2(Position { x: 0.0, y: 0.0 }, Velocity { x: 1.0, y: 2.0 });
        update_positions(&mut world);

        let pos = world.query::<&Position>().next().unwrap();
        assert_eq!(pos.x, 1.0);
        assert_eq!(pos.y, 2.0);
    }

    #[test]
    fn despawn_entity() {
        let mut world = World::new();
        let e = world.spawn((Position { x: 1.0, y: 2.0 },));
        assert!(world.despawn(e));
        let count = world.query::<&Position>().count();
        assert_eq!(count, 0);
    }

    #[test]
    fn query_filter_only_entities_with_all_components() {
        let mut world = World::new();
        world.spawn((Position { x: 1.0, y: 1.0 },));
        world.spawn2(Position { x: 2.0, y: 2.0 }, Velocity { x: 0.0, y: 0.0 });

        let mut iter = world.query::<(&Position, &Velocity)>();
        assert_eq!(iter.next().unwrap().0.x, 2.0);
        assert!(iter.next().is_none());
    }
}

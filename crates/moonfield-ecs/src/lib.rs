// The ECS crate: archetype-based storage with an archetype query engine. The
// public `World` is the archetype `World2` (see `world2`).
//
// The crate is still under active construction. The items silenced below are
// deliberate in-progress features kept for upcoming milestones (entity-ref /
// component-ref access, column-batch spawning, dynamic clone bundles, and the
// insert/remove edge tables for cross-archetype moves). The former sparse-set
// implementation has been fully removed.
#![allow(dead_code)]
#![allow(clippy::type_complexity)]

use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;

macro_rules! reverse_apply {
    ($m:ident [] $($reversed:tt)*) => {
        $m!{$($reversed),*} // base case
    };
    ($m:ident [$first:tt $($rest:tt)*] $($reversed:tt)*) => {
        reverse_apply!{$m [$($rest)*] $first $($reversed)*}
    };
}

/// Calls `m!()`, `m!(A)`, `m!(A, B)`, and `m!(A, B, C)` for i.e. `(m, A, B, C)`,
/// where `m` is any macro, for any number of parameters.
macro_rules! smaller_tuples_too {
    ($m:ident, $next:tt) => {
        $m!{}
        $m!{$next}
    };
    ($m:ident, $next:tt, $($rest:tt),*) => {
        smaller_tuples_too!{$m, $($rest),*}
        reverse_apply!{$m [$next $($rest)*]}
    };
}

mod archetype;
mod borrow;
mod bundle;
mod component;
mod component_ref;
mod entities;
mod entity_ref;
mod query;
mod resource;
mod system;
mod world2;

pub use component::Component;
pub use entities::Entity;
pub use query::Query;
pub use resource::Resource;
pub use system::{IntoSystem, System};
pub use world2::World2 as World;

/// Common ECS imports.
pub mod prelude {
    pub use crate::{Component, Entity, IntoSystem, Query, Resource, System, World};
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
        world.spawn((Position { x: 3.0, y: 4.0 }, Velocity { x: 0.5, y: 0.5 }));

        let positions: Vec<_> = world.query::<&Position>().map(|(_, v)| v.clone()).collect();
        assert_eq!(
            positions,
            vec![Position { x: 1.0, y: 2.0 }, Position { x: 3.0, y: 4.0 }]
        );
    }

    #[test]
    fn query_mutable() {
        let mut world = World::new();
        world.spawn((Position { x: 1.0, y: 2.0 }, Velocity { x: 1.0, y: 0.0 }));

        for (_, (pos, vel)) in world.query_mut::<(&mut Position, &Velocity)>() {
            pos.x += vel.x;
            pos.y += vel.y;
        }

        let pos = world
            .query::<&Position>()
            .map(|(_, v)| v.clone())
            .next()
            .unwrap();
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
    fn spawn_and_despawn() {
        let mut world = World::new();
        let e = world.spawn((Position { x: 10.0, y: 20.0 },));

        let pos: Vec<_> = world.query::<&Position>().map(|(_, v)| v.clone()).collect();
        assert_eq!(pos, vec![Position { x: 10.0, y: 20.0 }]);

        assert!(world.despawn(e).is_ok());
        let count = world.query::<&Position>().count();
        assert_eq!(count, 0);
    }

    #[test]
    fn system_runs_on_world() {
        fn update_positions(world: &mut World) {
            for (_, (pos, vel)) in world.query_mut::<(&mut Position, &Velocity)>() {
                pos.x += vel.x;
                pos.y += vel.y;
            }
        }

        let mut world = World::new();
        world.spawn((Position { x: 0.0, y: 0.0 }, Velocity { x: 1.0, y: 2.0 }));
        update_positions(&mut world);

        let pos = world
            .query::<&Position>()
            .map(|(_, v)| v.clone())
            .next()
            .unwrap();
        assert_eq!(pos.x, 1.0);
        assert_eq!(pos.y, 2.0);
    }

    #[test]
    fn despawn_entity() {
        let mut world = World::new();
        let e = world.spawn((Position { x: 1.0, y: 2.0 },));
        assert!(world.despawn(e).is_ok());
        let count = world.query::<&Position>().count();
        assert_eq!(count, 0);
    }

    #[test]
    fn query_filter_only_entities_with_all_components() {
        let mut world = World::new();
        world.spawn((Position { x: 1.0, y: 1.0 },));
        world.spawn((Position { x: 2.0, y: 2.0 }, Velocity { x: 0.0, y: 0.0 }));

        let mut iter = world.query::<(&Position, &Velocity)>();
        let (_, (pos, _)) = iter.next().unwrap();
        assert_eq!(pos.x, 2.0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn component_access_by_entity() {
        let mut world = World::new();
        let e = world.spawn((Position { x: 1.0, y: 2.0 },));
        assert!(world.get_component::<Velocity>(e).is_none());

        // Insert a new component (cross-archetype move).
        assert!(world
            .insert_component(e, Velocity { x: 3.0, y: 4.0 })
            .is_some());
        assert_eq!(world.get_component::<Velocity>(e).map(|v| v.x), Some(3.0));

        // Mutate through get_component_mut.
        world.get_component_mut::<Velocity>(e).unwrap().x = 9.0;
        assert_eq!(world.get_component::<Velocity>(e).map(|v| v.x), Some(9.0));

        // Replace when already present (no archetype change).
        assert!(world
            .insert_component(e, Velocity { x: 5.0, y: 6.0 })
            .is_some());
        assert_eq!(world.get_component::<Velocity>(e).map(|v| v.x), Some(5.0));

        // Entity remains queryable with both components after the move.
        let mut iter = world.query::<(&Position, &Velocity)>();
        assert!(iter.next().is_some());
        assert!(iter.next().is_none());
    }
}

//! Query filters, ported from the reference implementation's
//! `bevy_ecs::query::filter` (architecture-level).
//!
//! Filters narrow which entities a [`Query`](crate::Query) iterates without
//! fetching the components: `Query<&Transform, With<MeshRenderer>>`,
//! `Query<&mut Transform, Without<ChildOf>>`, `Query<&T, Or<(With<A>,
//! With<B>)>>`. All of them are **archetypal** — decided once per archetype
//! by its component type set, never per entity — so filtering costs one
//! type-set check per archetype at iterator construction.
//!
//! Composition: a tuple `(F0, F1, …)` is the conjunction (every filter must
//! match); `Or<(F0, F1, …)>` is the disjunction; `()` matches everything.

use std::any::TypeId;
use std::marker::PhantomData;

use crate::Component;

/// A filter on a [`Query`](crate::Query)'s matches, evaluated per archetype.
///
/// Implemented for [`With<T>`], [`Without<T>`], [`Or<(…)>`](Or), tuples
/// (conjunction), and `()` (no filter).
pub trait QueryFilter {
    /// Whether an archetype whose component set is probed by
    /// `set_contains` matches this filter.
    #[doc(hidden)]
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool;
}

/// Matches entities that have component `T` (not fetched — presence only).
pub struct With<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for With<T> {
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
        set_contains(TypeId::of::<T>())
    }
}

/// Matches entities that do *not* have component `T`.
pub struct Without<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Without<T> {
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
        !set_contains(TypeId::of::<T>())
    }
}

/// Matches entities matching at least one of the tuple's filters:
/// `Or<(With<A>, With<B>)>`.
pub struct Or<T>(PhantomData<fn() -> T>);

macro_rules! impl_query_filter_tuple {
    ($($name:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)] // the empty-tuple expansion ignores the probe
        impl<$($name: QueryFilter),*> QueryFilter for ($($name,)*) {
            fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
                $($name::matches_component_set(set_contains) &&)* true
            }
        }

        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        impl<$($name: QueryFilter),*> QueryFilter for Or<($($name,)*)> {
            fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
                $($name::matches_component_set(set_contains) ||)* false
            }
        }
    };
}

smaller_tuples_too!(impl_query_filter_tuple, F0, F1, F2, F3, F4, F5, F6, F7);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entity, Query, Schedule, SystemParam, World};

    #[derive(Debug, PartialEq)]
    struct Pos(f32);
    struct Vel;
    struct Frozen;

    /// Build a world covering the archetype edge cases: entities with every
    /// combination of Pos/Vel/Frozen, plus an entity with none of them.
    fn filter_world() -> (World, EntityKinds) {
        let mut world = World::new();
        let pos_only = world.spawn((Pos(1.0),));
        let pos_vel = world.spawn((Pos(2.0), Vel));
        let pos_vel_frozen = world.spawn((Pos(3.0), Vel, Frozen));
        let pos_frozen = world.spawn((Pos(4.0), Frozen));
        let bare = world.spawn(());
        (
            world,
            EntityKinds {
                pos_only,
                pos_vel,
                pos_vel_frozen,
                pos_frozen,
                bare,
            },
        )
    }

    struct EntityKinds {
        pos_only: Entity,
        pos_vel: Entity,
        pos_vel_frozen: Entity,
        pos_frozen: Entity,
        bare: Entity,
    }

    fn collect_with<F: QueryFilter>(world: &World) -> Vec<Entity> {
        let mut entities: Vec<_> = world.query_filtered::<&Pos, F>().map(|(e, _)| e).collect();
        entities.sort_by_key(|e| e.to_bits());
        entities
    }

    #[test]
    fn test_with_filter() {
        let (world, k) = filter_world();
        let with_vel = collect_with::<With<Vel>>(&world);
        assert_eq!(with_vel, vec![k.pos_vel, k.pos_vel_frozen]);

        let with_frozen = collect_with::<With<Frozen>>(&world);
        assert_eq!(with_frozen, vec![k.pos_vel_frozen, k.pos_frozen]);
    }

    #[test]
    fn test_without_filter() {
        let (world, k) = filter_world();
        let no_vel = collect_with::<Without<Vel>>(&world);
        assert_eq!(no_vel, vec![k.pos_only, k.pos_frozen]);
    }

    #[test]
    fn test_or_filter() {
        let (world, k) = filter_world();
        let vel_or_frozen = collect_with::<Or<(With<Vel>, With<Frozen>)>>(&world);
        assert_eq!(
            vel_or_frozen,
            vec![k.pos_vel, k.pos_vel_frozen, k.pos_frozen]
        );
    }

    #[test]
    fn test_tuple_filter_is_conjunction() {
        let (world, k) = filter_world();
        let both = collect_with::<(With<Vel>, Without<Frozen>)>(&world);
        assert_eq!(both, vec![k.pos_vel]);
    }

    #[test]
    fn test_unit_filter_matches_all_query_matches() {
        let (world, _k) = filter_world();
        // () = no filtering: every Pos entity, but not the bare entity
        // (the query item itself still restricts matches).
        let all = collect_with::<()>(&world);
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn test_query_param_with_filter_in_schedule() {
        #[derive(Default)]
        struct Count(u32);

        fn count_frozen(query: Query<&Pos, With<Frozen>>, mut count: crate::ResMut<Count>) {
            count.0 = query.iter().count() as u32;
        }

        let (mut world, _k) = filter_world();
        world.insert_resource(Count::default());
        let mut schedule = Schedule::new();
        schedule.add_systems(count_frozen);
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Count>().unwrap().0, 2);
    }

    #[test]
    fn test_query_get_respects_filter() {
        let (mut world, k) = filter_world();
        let _ = &mut world;
        // Filtered out by With<Vel>: pos_only has no Vel.
        let query = <Query<&Pos, With<Vel>> as SystemParam>::fetch(&world, &mut ());
        assert!(query.get(k.pos_only).is_none());
        assert!(query.get(k.pos_vel).is_some());
        // A bare entity (no Pos at all) is rejected by the query item itself.
        assert!(query.get(k.bare).is_none());
    }

    #[test]
    fn test_filtered_mutable_iteration() {
        let (mut world, _k) = filter_world();
        // Only Frozen entities get their Pos doubled.
        for (_, mut pos) in world.query_filtered_mut::<&mut Pos, With<Frozen>>() {
            pos.0 *= 2.0;
        }
        let frozen_vals: Vec<f32> = collect_with::<With<Frozen>>(&world)
            .iter()
            .map(|&e| world.get_component::<Pos>(e).unwrap().0)
            .collect();
        assert_eq!(frozen_vals, [6.0, 8.0]);
        let unfrozen = world.get_component::<Pos>(collect_with::<Without<Frozen>>(&world)[0]);
        assert_eq!(unfrozen.unwrap().0, 1.0);
    }
}

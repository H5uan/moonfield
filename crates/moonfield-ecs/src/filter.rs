//! Query filters, ported from the reference implementation's
//! `bevy_ecs::query::filter` (architecture-level).
//!
//! Filters narrow which entities a [`Query`](crate::Query) iterates without
//! fetching the components: `Query<&Transform, With<MeshRenderer>>`,
//! `Query<&mut Transform, Without<ChildOf>>`, `Query<&T, Or<(With<A>,
//! With<B>)>>`, `Query<&Transform, Changed<Transform>>`.
//!
//! Every filter has an **archetypal** part — decided once per archetype by
//! its component type set ([`QueryFilter::matches_component_set`]) — which is
//! what the query's archetype cache memorizes. `With`/`Without`/`Or` are
//! purely archetypal; `Added`/`Changed` additionally carry a **per-row**
//! part: the component's tick column is compared against the querying
//! system's `(last_run, this_run)` window at iteration time
//! ([`QueryFilter::build_row_state`] / [`QueryFilter::row_matches`]).
//!
//! Composition: a tuple `(F0, F1, …)` is the conjunction (every filter must
//! match); `Or<(F0, F1, …)>` is the disjunction; `()` matches everything.

use std::any::TypeId;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::Component;
use crate::archetype::Archetype;
use crate::change_detection::{ComponentTicks, Tick};
use crate::query::AccessRegistry;

/// A filter on a [`Query`](crate::Query)'s matches.
///
/// Implemented for [`With<T>`], [`Without<T>`], [`Added<T>`], [`Changed<T>`],
/// [`Or<(…)>`](Or), tuples (conjunction), and `()` (no filter).
pub trait QueryFilter {
    /// Whether an archetype whose component set is probed by
    /// `set_contains` matches this filter. This is the archetypal part: it
    /// decides which archetypes the query cache memorizes.
    #[doc(hidden)]
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool;

    /// Register the component access the filter's per-row evaluation
    /// performs (tick filters read their component's tick column), panicking
    /// on a conflict with already-live access. Called alongside the query
    /// elements' registration when a `Query` param is fetched. Default: the
    /// filter reads nothing.
    #[doc(hidden)]
    fn register_access(registry: &mut AccessRegistry) {
        let _ = registry;
    }

    /// Undo one [`Self::register_access`] call. Called from `Query`'s `Drop`,
    /// so it must not panic.
    #[doc(hidden)]
    fn unregister_access(registry: &mut AccessRegistry) {
        let _ = registry;
    }

    /// Per-archetype state for the per-row part of the filter, built by
    /// [`Self::build_row_state`] when an iterator is constructed over the
    /// archetype. Purely archetypal filters use `()`.
    #[doc(hidden)]
    type RowState<'w>: 'w
    where
        Self: 'w;

    /// Build the row state for archetype `a`, which
    /// [`Self::matches_component_set`] accepted. `last_run`/`this_run` are the
    /// window tick filters compare against (the querying system's window for
    /// `Query` params, the world's default window for imperative queries).
    #[doc(hidden)]
    fn build_row_state<'w>(a: &'w Archetype, last_run: Tick, this_run: Tick) -> Self::RowState<'w>
    where
        Self: 'w;

    /// The per-row predicate: whether `row` passes the filter's per-row part.
    /// Purely archetypal filters return `true` unconditionally. Only called
    /// for rows of archetypes [`Self::matches_component_set`] accepted.
    #[doc(hidden)]
    fn row_matches(state: &Self::RowState<'_>, row: u32) -> bool;
}

/// Matches entities that have component `T` (not fetched — presence only).
pub struct With<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for With<T> {
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
        set_contains(TypeId::of::<T>())
    }

    type RowState<'w>
        = ()
    where
        Self: 'w;

    fn build_row_state<'w>(_a: &'w Archetype, _last_run: Tick, _this_run: Tick)
    where
        Self: 'w,
    {
    }

    fn row_matches(_state: &(), _row: u32) -> bool {
        true
    }
}

/// Matches entities that do *not* have component `T`.
pub struct Without<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Without<T> {
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
        !set_contains(TypeId::of::<T>())
    }

    type RowState<'w>
        = ()
    where
        Self: 'w;

    fn build_row_state<'w>(_a: &'w Archetype, _last_run: Tick, _this_run: Tick)
    where
        Self: 'w,
    {
    }

    fn row_matches(_state: &(), _row: u32) -> bool {
        true
    }
}

/// Per-archetype state of a tick filter ([`Added`]/[`Changed`]): the
/// component's tick column plus the window to compare against.
///
/// `None` when the archetype has no `T` column, which can only happen under
/// [`Or`] (a sibling member supplied the archetype match); the row predicate
/// then fails.
#[doc(hidden)]
pub struct TickFetch<'w> {
    ticks: NonNull<ComponentTicks>,
    last_run: Tick,
    this_run: Tick,
    _marker: PhantomData<&'w ComponentTicks>,
}

impl<'w> TickFetch<'w> {
    fn build<T: Component>(a: &'w Archetype, last_run: Tick, this_run: Tick) -> Option<Self> {
        let column = a.get_state::<T>()?;
        Some(Self {
            // SAFETY: `column` is in bounds (just looked up); the tick array
            // is allocated alongside the column's row capacity, and the
            // archetype borrow outlives 'w.
            ticks: unsafe { a.ticks_base(column) },
            last_run,
            this_run,
            _marker: PhantomData,
        })
    }

    /// The `T` ticks of `row`, compared against the window by `which`
    /// (selecting the added or the changed tick).
    fn row_matches(&self, row: u32, which: fn(&ComponentTicks) -> Tick) -> bool {
        // SAFETY: `row` is within the archetype's length (the iterator only
        // probes live rows), so it is within the tick array's capacity.
        let ticks = unsafe { &*self.ticks.as_ptr().add(row as usize) };
        which(ticks).is_newer_than(self.last_run, self.this_run)
    }
}

/// Matches entities whose `T` component was added within the window:
/// `Query<&Transform, Added<Transform>>`.
///
/// The archetypal part requires `T`; the per-row part compares the
/// component's added tick against the querying system's window.
pub struct Added<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Added<T> {
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
        set_contains(TypeId::of::<T>())
    }

    fn register_access(registry: &mut AccessRegistry) {
        // Per-row evaluation reads T's tick column.
        registry.register_read(TypeId::of::<T>(), std::any::type_name::<T>());
    }

    fn unregister_access(registry: &mut AccessRegistry) {
        registry.unregister_read(TypeId::of::<T>());
    }

    type RowState<'w>
        = Option<TickFetch<'w>>
    where
        Self: 'w;

    fn build_row_state<'w>(a: &'w Archetype, last_run: Tick, this_run: Tick) -> Self::RowState<'w>
    where
        Self: 'w,
    {
        TickFetch::build::<T>(a, last_run, this_run)
    }

    fn row_matches(state: &Self::RowState<'_>, row: u32) -> bool {
        state
            .as_ref()
            .is_some_and(|fetch| fetch.row_matches(row, |ticks| ticks.added))
    }
}

/// Matches entities whose `T` component was mutably accessed within the
/// window: `Query<&Transform, Changed<Transform>>`. Adding a component also
/// sets its changed tick, so freshly added components match.
///
/// The archetypal part requires `T`; the per-row part compares the
/// component's changed tick against the querying system's window.
pub struct Changed<T: Component>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Changed<T> {
    fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
        set_contains(TypeId::of::<T>())
    }

    fn register_access(registry: &mut AccessRegistry) {
        // Per-row evaluation reads T's tick column.
        registry.register_read(TypeId::of::<T>(), std::any::type_name::<T>());
    }

    fn unregister_access(registry: &mut AccessRegistry) {
        registry.unregister_read(TypeId::of::<T>());
    }

    type RowState<'w>
        = Option<TickFetch<'w>>
    where
        Self: 'w;

    fn build_row_state<'w>(a: &'w Archetype, last_run: Tick, this_run: Tick) -> Self::RowState<'w>
    where
        Self: 'w,
    {
        TickFetch::build::<T>(a, last_run, this_run)
    }

    fn row_matches(state: &Self::RowState<'_>, row: u32) -> bool {
        state
            .as_ref()
            .is_some_and(|fetch| fetch.row_matches(row, |ticks| ticks.changed))
    }
}

/// Matches entities matching at least one of the tuple's filters:
/// `Or<(With<A>, With<B>)>`.
pub struct Or<T>(PhantomData<fn() -> T>);

macro_rules! impl_query_filter_tuple {
    ($($name:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)] // the empty-tuple expansion ignores the probe
        #[allow(clippy::unused_unit)] // the empty-tuple expansion produces `()`
        impl<$($name: QueryFilter),*> QueryFilter for ($($name,)*) {
            fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
                $($name::matches_component_set(set_contains) &&)* true
            }

            #[allow(unused_variables)] // the empty-tuple expansion ignores it
            fn register_access(registry: &mut AccessRegistry) {
                $($name::register_access(registry);)*
            }

            #[allow(unused_variables)]
            fn unregister_access(registry: &mut AccessRegistry) {
                $($name::unregister_access(registry);)*
            }

            type RowState<'w> = ($($name::RowState<'w>,)*) where Self: 'w;

            #[allow(unused_variables)] // the empty-tuple expansion ignores them
            fn build_row_state<'w>(
                a: &'w Archetype,
                last_run: Tick,
                this_run: Tick,
            ) -> Self::RowState<'w>
            where
                Self: 'w,
            {
                ($($name::build_row_state(a, last_run, this_run),)*)
            }

            #[allow(unused_variables)] // the empty-tuple expansion ignores them
            fn row_matches(state: &Self::RowState<'_>, row: u32) -> bool {
                let ($($name,)*) = state;
                $($name::row_matches($name, row) &&)* true
            }
        }

        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(clippy::unused_unit)] // the empty-tuple expansion produces `()`
        impl<$($name: QueryFilter),*> QueryFilter for Or<($($name,)*)> {
            fn matches_component_set(set_contains: &dyn Fn(TypeId) -> bool) -> bool {
                $($name::matches_component_set(set_contains) ||)* false
            }

            fn register_access(registry: &mut AccessRegistry) {
                $($name::register_access(registry);)*
            }

            fn unregister_access(registry: &mut AccessRegistry) {
                $($name::unregister_access(registry);)*
            }

            type RowState<'w> = ($($name::RowState<'w>,)*) where Self: 'w;

            fn build_row_state<'w>(
                a: &'w Archetype,
                last_run: Tick,
                this_run: Tick,
            ) -> Self::RowState<'w>
            where
                Self: 'w,
            {
                ($($name::build_row_state(a, last_run, this_run),)*)
            }

            fn row_matches(state: &Self::RowState<'_>, row: u32) -> bool {
                let ($($name,)*) = state;
                $($name::row_matches($name, row) ||)* false
            }
        }
    };
}

smaller_tuples_too!(impl_query_filter_tuple, F0, F1, F2, F3, F4, F5, F6, F7);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entity, Query, Schedule, SystemState, World};

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
    fn test_added_and_changed_filters_use_the_world_window() {
        let (mut world, k) = filter_world();
        // Freshly spawned: everything is added (and changed).
        assert_eq!(collect_with::<Added<Pos>>(&world).len(), 4);
        assert_eq!(collect_with::<Changed<Pos>>(&world).len(), 4);

        // Advance the clock: nothing is new within the new window.
        world.increment_change_tick();
        assert_eq!(collect_with::<Added<Pos>>(&world).len(), 0);
        assert_eq!(collect_with::<Changed<Pos>>(&world).len(), 0);

        // A write marks only the written entity changed, not added.
        world.get_component_mut::<Pos>(k.pos_only).unwrap().0 = 9.0;
        assert_eq!(collect_with::<Changed<Pos>>(&world), vec![k.pos_only]);
        assert_eq!(collect_with::<Added<Pos>>(&world).len(), 0);
    }

    #[test]
    fn test_or_of_tick_filters_matches_either_tick() {
        let mut world = World::new();
        let a = world.spawn((Pos(1.0),));
        let _b = world.spawn((Pos(2.0), Vel));
        world.increment_change_tick();

        // Rewrite Pos on `a` (changed, not added); nothing on `b`.
        world.insert_component(a, Pos(9.0));
        let changed_or_vel = collect_with::<Or<(Changed<Pos>, Added<Vel>)>>(&world);
        assert_eq!(changed_or_vel, vec![a]);

        // A fresh spawn of a Vel entity fires its added tick (replacing an
        // existing component bumps only its changed tick, so a rewrite of
        // `b`'s Vel would not qualify).
        let c = world.spawn((Pos(3.0), Vel));
        let changed_or_vel = collect_with::<Or<(Changed<Pos>, Added<Vel>)>>(&world);
        assert_eq!(changed_or_vel, vec![a, c]);
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
        let mut state = SystemState::<Query<&Pos, With<Vel>>>::new();
        let query = state.get(&world);
        assert!(query.get(k.pos_only).is_none());
        assert!(query.get(k.pos_vel).is_some());
        // A bare entity (no Pos at all) is rejected by the query item itself.
        assert!(query.get(k.bare).is_none());
    }

    #[test]
    fn test_query_get_applies_tick_filters_per_row() {
        let (mut world, k) = filter_world();
        // The world-window clock: after an advance, nothing is changed.
        world.increment_change_tick();
        world.get_component_mut::<Pos>(k.pos_vel).unwrap().0 = 9.0;

        let mut state = SystemState::<Query<&Pos, Changed<Pos>>>::new();
        // The first get's window starts at tick 0: every row passes.
        {
            let query = state.get(&world);
            assert!(query.get(k.pos_only).is_some());
        }
        // The next get opens a fresh window covering only the ticks since —
        // both rows are filtered out, including the one written just before
        // the first get.
        let query = state.get(&world);
        assert!(query.get(k.pos_only).is_none());
        assert!(query.get(k.pos_vel).is_none());
    }

    #[test]
    fn test_tick_filters_use_each_systems_own_window() {
        #[derive(Default)]
        struct EveryFrame(Vec<usize>);
        #[derive(Default)]
        struct Sparse(Vec<usize>);

        fn every(query: Query<&Pos, Added<Pos>>, mut log: crate::ResMut<EveryFrame>) {
            log.0.push(query.iter().count());
        }
        fn sparse(query: Query<&Pos, Added<Pos>>, mut log: crate::ResMut<Sparse>) {
            log.0.push(query.iter().count());
        }

        let mut world = World::new();
        world.insert_resource(EveryFrame::default());
        world.insert_resource(Sparse::default());
        let mut schedule = Schedule::new();
        schedule.add_systems(every);
        let mut sparse = crate::IntoSystem::into_system(sparse);

        // Run 1: nothing exists yet; the first spawn lands after it.
        schedule.run(&mut world);
        let _a = world.spawn((Pos(1.0),));
        // Run 2: `every` sees the new entity; `sparse`'s first window (from
        // tick 0) sees it too.
        schedule.run(&mut world);
        sparse.run(&mut world);
        // Two more spawns, one schedule run apart; `sparse` sits run 3 out.
        let _b = world.spawn((Pos(2.0),));
        schedule.run(&mut world);
        let _c = world.spawn((Pos(3.0),));
        schedule.run(&mut world);
        sparse.run(&mut world);

        // `every` saw exactly the additions since its previous run each time;
        // `sparse`'s second window spans both idle runs, so it sees both
        // entities the per-run system saw one at a time.
        assert_eq!(world.get_resource::<EveryFrame>().unwrap().0, [0, 1, 1, 1]);
        assert_eq!(world.get_resource::<Sparse>().unwrap().0, [1, 2]);
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

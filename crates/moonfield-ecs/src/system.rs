//! Systems and system parameters.
//!
//! Ported mechanism from Bevy's `bevy_ecs::system` (architecture-level, not
//! API-complete): a system is an ordinary function or closure whose
//! parameters declare how it accesses the world. Each parameter type
//! implements [`SystemParam`]; the function is wrapped into a [`System`] by
//! [`IntoSystem`] and run by a [`Schedule`](crate::Schedule).
//!
//! ```ignore
//! fn physics(time: Res<DeltaTime>, mut query: Query<(&mut Position, &Velocity)>) {
//!     for (_, (mut pos, vel)) in query.iter_mut() {
//!         pos.x += vel.x * time.0;
//!     }
//! }
//! ```
//!
//! Exclusive systems — `FnMut(&mut World)` — remain supported for code that
//! needs unrestricted world access, mirroring Bevy's exclusive systems.
//!
//! Every system run advances the world's change tick once
//! ([`World::increment_change_tick`]): the run's tick is the returned one and
//! the world counter moves on to the next run's, so writes made after a run —
//! by later systems in the same schedule, applied commands, or world
//! accessors — record a strictly newer tick than everything the run observed.
//! A system's queries therefore compare against its own `(last_run,
//! this_run)` window. Schedule set anchors observe nothing and do not
//! advance the tick.

use std::any::type_name;
use std::cell::{Ref, RefMut};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::{
    Entity, Resource, World,
    change_detection::Tick,
    filter::QueryFilter,
    query::{QueryGetGuard, QueryIter, WorldQuery},
};

/// A unit of work that operates on a [`World`].
///
/// Systems are single-threaded: they run one at a time on the main thread,
/// each with exclusive access to the world for the duration of its run.
pub trait System: Send + Sync + 'static {
    /// The system's name — the type name of the underlying function or
    /// closure. Used as the default label for ordering constraints.
    fn name(&self) -> &str;

    /// Run the system once against `world`.
    fn run(&mut self, world: &mut World);

    /// Clamp this system's change-detection window when it has aged past
    /// [`Tick::MAX`] relative to `present`. Called by the world's periodic
    /// tick check; systems without a window keep the no-op default.
    fn check_change_ticks(&mut self, _present: Tick) {}
}

impl System for Box<dyn System> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn run(&mut self, world: &mut World) {
        (**self).run(world);
    }

    fn check_change_ticks(&mut self, present: Tick) {
        (**self).check_change_ticks(present);
    }
}

/// Trait for types that can be turned into a boxed [`System`].
///
/// Implemented for:
/// - functions/closures taking [`SystemParam`]s, e.g.
///   `fn(Res<A>, Query<&B>)` (via [`SystemParamFunction`]);
/// - exclusive systems `FnMut(&mut World)` with unrestricted world access.
///
/// The `Marker` type parameter keeps the two implementations disjoint for
/// type inference (Bevy uses the same trick).
pub trait IntoSystem<Marker>: Send + Sync + 'static {
    /// Convert into a boxed system.
    fn into_system(self) -> Box<dyn System>;
}

// ---------------------------------------------------------------------
// SystemParam
// ---------------------------------------------------------------------

/// A type that can be used as a parameter of a function system.
///
/// `State` is per-system data that persists across runs (e.g. a [`Local`]'s
/// value); `Item` is the value actually passed to the system function on each
/// run, borrowing the world for `'w` and the state for `'s`.
///
/// Params are fetched from a *shared* world borrow so that several params can
/// coexist in one system; safety is enforced dynamically (resources via
/// `RefCell`, component columns via archetype borrow flags), so conflicting
/// params panic at runtime instead of failing to compile.
pub trait SystemParam: Sized {
    /// Per-system persistent state.
    type State: Send + Sync + 'static;
    /// The value passed to the system function for one run.
    type Item<'w, 's>;
    /// Create the initial state.
    fn init_state() -> Self::State;
    /// Refresh the change-detection window this param's state carries for
    /// the run about to fetch: `last_run` is the owning system's previous
    /// run tick, `this_run` the tick of the run being fetched. Params whose
    /// state carries no window keep the no-op default.
    fn refresh_window(_state: &mut Self::State, _last_run: Tick, _this_run: Tick) {}
    /// Fetch the parameter for one run from `world` and `state`.
    fn fetch<'w, 's>(world: &'w World, state: &'s mut Self::State) -> Self::Item<'w, 's>;
}

/// Shorthand for the per-run value of a [`SystemParam`].
pub type SystemParamItem<'w, 's, P> = <P as SystemParam>::Item<'w, 's>;

/// A reusable container for a [`SystemParam`]'s persistent state, decoupled
/// from any one system: [`SystemState::new`] initializes the state once and
/// [`SystemState::get`] fetches the param for one run. Code that is not a
/// function system — an exclusive system's helper, a render command — uses it
/// to hold param state across calls the way a function system does internally.
pub struct SystemState<P: SystemParam> {
    state: P::State,
    /// The tick of the latest `get` call — the start of the next window.
    last_run: Tick,
}

impl<P: SystemParam> SystemState<P> {
    /// Initialize the state for `P` (`Local` defaults and the like).
    pub fn new() -> Self {
        Self {
            state: P::init_state(),
            last_run: Tick::new(0),
        }
    }

    /// Fetch the param value for one run against `world`.
    ///
    /// Each call follows the function-system contract: it advances the
    /// world's change tick once and opens a fresh change-detection window,
    /// so tick-aware params report the changes made since the previous call.
    pub fn get<'w, 's>(&'s mut self, world: &'w World) -> SystemParamItem<'w, 's, P> {
        let this_run = world.increment_change_tick();
        P::refresh_window(&mut self.state, self.last_run, this_run);
        self.last_run = this_run;
        P::fetch(world, &mut self.state)
    }
}

impl<P: SystemParam> Default for SystemState<P> {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! impl_system_param_tuple {
    ($($param:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(clippy::unused_unit)] // the empty-tuple expansion produces `()`
        impl<$($param: SystemParam),*> SystemParam for ($($param,)*) {
            type State = ($($param::State,)*);
            type Item<'w, 's> = ($($param::Item<'w, 's>,)*);

            fn init_state() -> Self::State {
                ($($param::init_state(),)*)
            }

            #[allow(unused_variables)]
            fn refresh_window(state: &mut Self::State, last_run: Tick, this_run: Tick) {
                let ($($param,)*) = state;
                $($param::refresh_window($param, last_run, this_run);)*
            }

            #[allow(unused_variables)]
            fn fetch<'w, 's>(
                world: &'w World,
                state: &'s mut Self::State,
            ) -> Self::Item<'w, 's> {
                let ($($param,)*) = state;
                ($($param::fetch(world, $param),)*)
            }
        }
    };
}

smaller_tuples_too!(impl_system_param_tuple, P0, P1, P2, P3, P4, P5, P6, P7);

// ---------------------------------------------------------------------
// Res / ResMut
// ---------------------------------------------------------------------

/// Shared access to a resource of type `T` as a system param.
///
/// Panics when fetched if the resource does not exist; use
/// `Option<Res<T>>` for optional access.
pub struct Res<'w, T: Resource> {
    value: Ref<'w, T>,
}

impl<T: Resource> Deref for Res<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: Resource> SystemParam for Res<'_, T> {
    type State = ();
    type Item<'w, 's> = Res<'w, T>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        Res {
            value: world
                .get_resource::<T>()
                .unwrap_or_else(|| panic!("resource `{}` does not exist", type_name::<T>())),
        }
    }
}

/// Unique access to a resource of type `T` as a system param.
///
/// Panics when fetched if the resource does not exist; use
/// `Option<ResMut<T>>` for optional access.
pub struct ResMut<'w, T: Resource> {
    value: RefMut<'w, T>,
}

impl<T: Resource> Deref for ResMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: Resource> DerefMut for ResMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: Resource> SystemParam for ResMut<'_, T> {
    type State = ();
    type Item<'w, 's> = ResMut<'w, T>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        ResMut {
            value: world
                .get_resource_mut::<T>()
                .unwrap_or_else(|| panic!("resource `{}` does not exist", type_name::<T>())),
        }
    }
}

impl<T: Resource> SystemParam for Option<Res<'_, T>> {
    type State = ();
    type Item<'w, 's> = Option<Res<'w, T>>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        world.get_resource::<T>().map(|value| Res { value })
    }
}

impl<T: Resource> SystemParam for Option<ResMut<'_, T>> {
    type State = ();
    type Item<'w, 's> = Option<ResMut<'w, T>>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        world.get_resource_mut::<T>().map(|value| ResMut { value })
    }
}

// ---------------------------------------------------------------------
// Local
// ---------------------------------------------------------------------

/// Per-system local state as a system param, initialized with
/// [`Default::default`] and persisted across runs of that system.
pub struct Local<'s, T: Send + Sync + 'static>(&'s mut T);

impl<T: Send + Sync + 'static> Deref for Local<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<T: Send + Sync + 'static> DerefMut for Local<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

impl<T: Default + Send + Sync + 'static> SystemParam for Local<'_, T> {
    type State = T;
    type Item<'w, 's> = Local<'s, T>;

    fn init_state() -> Self::State {
        T::default()
    }

    fn fetch<'w, 's>(_world: &'w World, state: &'s mut Self::State) -> Self::Item<'w, 's> {
        Local(state)
    }
}

// ---------------------------------------------------------------------
// Query (system param)
// ---------------------------------------------------------------------

/// Component query as a system param, over the archetype [`WorldQuery`]
/// machinery, with an optional archetype filter `F`
/// ([`With`](crate::With)/[`Without`](crate::Without)/[`Or`](crate::Or)).
///
/// ```ignore
/// fn integrate(mut query: Query<(&mut Position, &Velocity), Without<Frozen>>) {
///     for (_, (mut pos, vel)) in query.iter_mut() {
///         pos.x += vel.x;
///     }
/// }
/// ```
///
/// Conflicting access (e.g. two live iterators over the same mutable column)
/// is caught by the archetype borrow flags and panics, exactly like
/// [`World::query_mut`].
pub struct Query<'w, Q: WorldQuery, F: QueryFilter = ()> {
    world: &'w World,
    window: QueryWindow,
    _marker: PhantomData<fn() -> (Q, F)>,
}

/// The change-detection window a [`Query`] param's state carries between
/// runs: the owning system's `last_run` and the tick of the run being
/// fetched. The system runner refreshes it before every fetch.
#[derive(Copy, Clone, Debug)]
pub struct QueryWindow {
    /// The tick of the system's previous completed run.
    pub(crate) last_run: Tick,
    /// The tick of the run being fetched.
    pub(crate) this_run: Tick,
}

impl<'w, Q: WorldQuery, F: QueryFilter> Query<'w, Q, F> {
    /// Iterate all matching entities with shared access.
    pub fn iter(&self) -> QueryIter<'_, Q> {
        QueryIter::new_shared(
            self.world,
            &archetype_matches::<F>,
            self.window.last_run,
            self.window.this_run,
        )
    }

    /// Iterate all matching entities with mutable access.
    pub fn iter_mut(&mut self) -> QueryIter<'_, Q> {
        // SAFETY: the returned iterator and the items it yields borrow this
        // `Query` mutably, so no second mutable iterator can be created from
        // it while they are alive; the running system holds the world's only
        // access. Conflicting columns across *different* params are still
        // caught by the archetype borrow flags.
        unsafe {
            QueryIter::new(
                self.world,
                &archetype_matches::<F>,
                self.window.last_run,
                self.window.this_run,
            )
        }
    }

    /// Fetch the item for a single entity, if it matches the query *and* the
    /// filter.
    ///
    /// Returns a [`QueryGetGuard`](crate::QueryGetGuard) holding the item
    /// and its column borrows for any query shape — tuples and `Option`
    /// compose exactly as in iteration. Mutable elements go through
    /// `DerefMut` and mark change ticks like iteration does.
    pub fn get(&self, entity: Entity) -> Option<QueryGetGuard<'_, Q>> {
        let (arch_i, _) = self.world.locate_entity(entity)?;
        if !archetype_matches::<F>(&self.world.raw_archetypes()[arch_i]) {
            return None;
        }
        Q::get_entity(
            self.world,
            entity,
            self.window.last_run,
            self.window.this_run,
        )
    }
}

/// The `QueryFilter` evaluation bridge: an archetype matches if its component
/// type set satisfies `F`.
pub(crate) fn archetype_matches<F: QueryFilter>(archetype: &crate::archetype::Archetype) -> bool {
    F::matches_component_set(&|t| archetype.has_in_runtime(t))
}

impl<Q: WorldQuery, F: QueryFilter> SystemParam for Query<'_, Q, F> {
    type State = QueryWindow;
    type Item<'w, 's> = Query<'w, Q, F>;

    fn init_state() -> Self::State {
        // last_run 0: a system's first run observes every component as new.
        QueryWindow {
            last_run: Tick::new(0),
            this_run: Tick::new(0),
        }
    }

    fn refresh_window(state: &mut Self::State, last_run: Tick, this_run: Tick) {
        *state = QueryWindow { last_run, this_run };
    }

    fn fetch<'w, 's>(world: &'w World, state: &'s mut Self::State) -> Self::Item<'w, 's> {
        Query {
            world,
            window: *state,
            _marker: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------
// SystemParamFunction: FnMut(params...) as a system
// ---------------------------------------------------------------------

/// A function or closure whose parameters are all [`SystemParam`]s, and which
/// can therefore run as a system. Ported from Bevy's `SystemParamFunction`,
/// including the `for<'a> &'a mut Func` bound shape that `rustc` needs to
/// accept the higher-ranked lifetimes.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid system",
    label = "invalid system"
)]
pub trait SystemParamFunction<Marker>: Send + Sync + 'static {
    /// The [`SystemParam`]s of this system, as a tuple.
    type Param: SystemParam + 'static;

    /// Executes this system once with fetched params.
    fn run(&mut self, param_value: SystemParamItem<Self::Param>);
}

macro_rules! impl_system_function {
    ($($param:ident),*) => {
        #[allow(non_snake_case)]
        impl<Func, $($param: SystemParam + 'static),*> SystemParamFunction<fn($($param,)*)> for Func
        where
            Func: Send + Sync + 'static,
            for<'a> &'a mut Func:
                FnMut($($param,)*) +
                FnMut($(SystemParamItem<$param>),*),
        {
            type Param = ($($param,)*);

            #[inline]
            fn run(&mut self, param_value: SystemParamItem<($($param,)*)>) {
                // Yes, this is strange, but `rustc` fails to compile this impl
                // without using this function (same as Bevy). It fails to
                // recognize that `func` is a function, potentially because of
                // the multiple impls of `FnMut`.
                #[allow(clippy::too_many_arguments)]
                fn call_inner<$($param,)*>(
                    mut f: impl FnMut($($param,)*),
                    $($param: $param,)*
                ) {
                    f($($param,)*)
                }
                let ($($param,)*) = param_value;
                call_inner(self, $($param,)*)
            }
        }
    };
}

smaller_tuples_too!(impl_system_function, P0, P1, P2, P3, P4, P5, P6, P7);

// ---------------------------------------------------------------------
// IntoSystem implementations
// ---------------------------------------------------------------------

/// Marker for [`IntoSystem`] implementations of param-based function systems.
pub struct FunctionSystemMarker<M>(PhantomData<fn() -> M>);

/// Marker for [`IntoSystem`] implementations of exclusive systems.
pub struct ExclusiveSystemMarker;

impl<F, M> IntoSystem<FunctionSystemMarker<M>> for F
where
    F: SystemParamFunction<M>,
    M: 'static,
{
    fn into_system(self) -> Box<dyn System> {
        Box::new(FunctionSystem::<F, F::Param, M> {
            state: <F::Param as SystemParam>::init_state(),
            last_run: Tick::new(0),
            f: self,
            name: type_name::<F>(),
            _marker: PhantomData,
        })
    }
}

struct FunctionSystem<F, P: SystemParam, M> {
    f: F,
    state: P::State,
    /// The tick of this system's latest completed run — the start of its
    /// change-detection window.
    last_run: Tick,
    name: &'static str,
    _marker: PhantomData<fn() -> M>,
}

impl<F, M, P> System for FunctionSystem<F, P, M>
where
    F: SystemParamFunction<M, Param = P>,
    P: SystemParam + 'static,
    M: 'static,
{
    fn name(&self) -> &str {
        self.name
    }

    fn run(&mut self, world: &mut World) {
        // One system run = one tick: the returned tick is this run's, and
        // the world counter moves on to the next run's, so every write made
        // after this run — by later systems, applied commands, or world
        // accessors — records a strictly newer tick.
        let this_run = world.increment_change_tick();
        P::refresh_window(&mut self.state, self.last_run, this_run);
        let params = P::fetch(world, &mut self.state);
        SystemParamFunction::run(&mut self.f, params);
        self.last_run = this_run;
    }

    fn check_change_ticks(&mut self, present: Tick) {
        self.last_run.check_tick(present);
    }
}

impl<F> IntoSystem<ExclusiveSystemMarker> for F
where
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    fn into_system(self) -> Box<dyn System> {
        Box::new(ExclusiveSystem {
            f: self,
            name: type_name::<F>(),
        })
    }
}

struct ExclusiveSystem<F> {
    f: F,
    name: &'static str,
}

impl<F> System for ExclusiveSystem<F>
where
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        self.name
    }

    fn run(&mut self, world: &mut World) {
        // One system run = one tick, like a function system: an exclusive
        // system can write through world accessors, and those writes must
        // rank strictly newer than the previous run's observations.
        world.increment_change_tick();
        (self.f)(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commands, Schedule};

    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Velocity {
        x: f32,
    }

    #[derive(Debug, Default)]
    struct Log(Vec<String>);

    struct Delta(f32);

    #[test]
    fn test_system_params_resolve_and_mutate() {
        fn integrate(time: Res<Delta>, mut query: Query<(&mut Position, &Velocity)>) {
            for (_, (mut pos, vel)) in query.iter_mut() {
                pos.x += vel.x * time.0;
            }
        }

        let mut world = World::new();
        world.insert_resource(Delta(0.5));
        let entity = world.spawn((Position { x: 1.0 }, Velocity { x: 4.0 }));

        let mut system = IntoSystem::into_system(integrate);
        system.run(&mut world);
        assert_eq!(
            world.get_component::<Position>(entity),
            Some(&Position { x: 3.0 })
        );
    }

    #[test]
    fn test_option_res_is_none_when_missing() {
        fn probe(maybe: Option<Res<Delta>>, mut log: ResMut<Log>) {
            log.0.push(format!("{}", maybe.is_some()));
        }

        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut system = IntoSystem::into_system(probe);
        system.run(&mut world);
        world.insert_resource(Delta(1.0));
        system.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["false", "true"]);
    }

    #[test]
    fn test_local_persists_across_runs() {
        fn counter(mut n: Local<u32>, mut log: ResMut<Log>) {
            *n += 1;
            log.0.push(n.to_string());
        }

        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut system = IntoSystem::into_system(counter);
        system.run(&mut world);
        system.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["1", "2"]);
    }

    #[test]
    fn test_tuple_params() {
        fn all_at_once(
            time: Res<Delta>,
            mut log: ResMut<Log>,
            mut n: Local<u32>,
            query: Query<&Position>,
            commands: Commands,
        ) {
            *n += 1;
            log.0
                .push(format!("{} {} {}", time.0, query.iter().count(), *n));
            commands.spawn((Velocity { x: 1.0 },));
        }

        let mut world = World::new();
        world.insert_resource(Delta(2.0));
        world.insert_resource(Log::default());
        world.spawn((Position { x: 0.0 },));

        let mut system = IntoSystem::into_system(all_at_once);
        system.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["2 1 1"]);
        // Commands are only applied by the world/schedule, not by `run`.
        assert_eq!(world.query::<&Velocity>().count(), 0);
        world.apply_commands();
        assert_eq!(world.query::<&Velocity>().count(), 1);
    }

    #[test]
    fn test_exclusive_system() {
        let mut world = World::new();
        let mut system = IntoSystem::into_system(|world: &mut World| {
            world.spawn((Position { x: 9.0 },));
        });
        system.run(&mut world);
        assert_eq!(world.query::<&Position>().count(), 1);
    }

    #[test]
    fn test_system_state_caches_local_across_gets() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut state = SystemState::<(Local<u32>, ResMut<Log>)>::new();
        for expected in ["1", "2"] {
            let (mut n, mut log) = state.get(&world);
            *n += 1;
            log.0.push(n.to_string());
            assert_eq!(n.to_string(), expected);
        }
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["1", "2"]);
    }

    #[test]
    fn test_idle_system_sees_changes_made_while_idle() {
        // The fixed-update shape: `observer` runs every third frame while
        // `writer` runs every frame; writes made while `observer` was idle
        // must still be reported.
        fn writer(mut query: Query<&mut Position>) {
            for (_, mut pos) in query.iter_mut() {
                pos.x += 1.0;
            }
        }
        fn observer(mut query: Query<&mut Position>, mut log: ResMut<Log>) {
            let changed = query.iter_mut().any(|(_, pos)| pos.is_changed());
            log.0
                .push(if changed { "changed" } else { "unchanged" }.to_string());
        }

        let mut world = World::new();
        world.insert_resource(Log::default());
        world.spawn((Position { x: 0.0 },));
        let mut update = Schedule::new();
        update.add_systems(writer);
        let mut observer = IntoSystem::into_system(observer);

        for frame in 0..3 {
            update.run(&mut world);
            if frame == 2 {
                observer.run(&mut world);
            }
        }
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["changed"]);
    }

    #[test]
    fn test_reader_sees_later_writer_on_its_next_run() {
        // `reader` runs before `writer` in the same schedule, so the write
        // lands after `reader` fetched; `reader`'s next run must see it.
        fn reader(mut query: Query<&mut Position>, mut log: ResMut<Log>) {
            let changed = query.iter_mut().any(|(_, pos)| pos.is_changed());
            log.0
                .push(if changed { "changed" } else { "unchanged" }.to_string());
        }
        fn writer(mut step: Local<u32>, mut query: Query<&mut Position>) {
            if *step == 0 {
                for (_, mut pos) in query.iter_mut() {
                    pos.x += 1.0;
                }
            }
            *step += 1;
        }

        let mut world = World::new();
        world.insert_resource(Log::default());
        world.spawn((Position { x: 0.0 },));
        let mut schedule = Schedule::new();
        schedule.add_systems((reader, writer));
        schedule.run(&mut world);
        schedule.run(&mut world);
        schedule.run(&mut world);
        // Run 1: the spawn is new to `reader`. Run 2: `writer`'s run-1
        // write. Run 3: nothing new.
        assert_eq!(
            world.get_resource::<Log>().unwrap().0,
            ["changed", "changed", "unchanged"]
        );
    }

    #[test]
    fn test_system_state_windows_advance_per_get() {
        // `SystemState::get` follows the same one-run-one-tick contract as
        // a function system: each fetch opens a fresh window.
        let mut world = World::new();
        let entity = world.spawn((Position { x: 0.0 },));
        let mut state = SystemState::<Query<&mut Position>>::new();

        {
            let query = state.get(&world);
            let mut pos = query.get(entity).expect("the entity was just spawned");
            assert!(pos.is_changed()); // first window: everything is new
            pos.x += 1.0; // stamps the window's this_run
        }
        {
            let query = state.get(&world);
            let pos = query.get(entity).expect("the entity was just spawned");
            assert!(!pos.is_changed()); // own write is not re-reported
        }
    }
}

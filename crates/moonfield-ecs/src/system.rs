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

use std::any::type_name;
use std::cell::{Ref, RefMut};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::{query::WorldQuery, Entity, Resource, World};

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
}

impl System for Box<dyn System> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn run(&mut self, world: &mut World) {
        (**self).run(world);
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
    /// Fetch the parameter for one run from `world` and `state`.
    fn fetch<'w, 's>(world: &'w World, state: &'s mut Self::State) -> Self::Item<'w, 's>;
}

/// Shorthand for the per-run value of a [`SystemParam`].
pub type SystemParamItem<'w, 's, P> = <P as SystemParam>::Item<'w, 's>;

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
/// machinery.
///
/// ```ignore
/// fn integrate(mut query: Query<(&mut Position, &Velocity)>) {
///     for (_, (mut pos, vel)) in query.iter_mut() {
///         pos.x += vel.x;
///     }
/// }
/// ```
///
/// Conflicting access (e.g. two live iterators over the same mutable column)
/// is caught by the archetype borrow flags and panics, exactly like
/// [`World::query_mut`].
pub struct Query<'w, Q: WorldQuery> {
    world: &'w World,
    _marker: PhantomData<fn() -> Q>,
}

impl<'w, Q: WorldQuery> Query<'w, Q> {
    /// Iterate all matching entities with shared access.
    pub fn iter(&self) -> Q::Iter<'_> {
        Q::fetch(self.world)
    }

    /// Iterate all matching entities with mutable access.
    pub fn iter_mut(&mut self) -> Q::Iter<'_> {
        Q::fetch_mut_cell(self.world)
    }

    /// Fetch the item for a single entity, if it matches the query.
    ///
    /// Returns a guard that releases the column's borrow flag on drop
    /// ([`EntityRef`](crate::EntityRef) for `&T` queries,
    /// [`EntityMut`](crate::EntityMut) for `&mut T` queries). Only the
    /// single-component query shapes support per-entity access for now.
    pub fn get(&self, entity: Entity) -> Option<Q::EntityFetch<'_>> {
        Q::get_entity(self.world, entity)
    }
}

impl<Q: WorldQuery> SystemParam for Query<'_, Q> {
    type State = ();
    type Item<'w, 's> = Query<'w, Q>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        Query {
            world,
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
            f: self,
            name: type_name::<F>(),
            _marker: PhantomData,
        })
    }
}

struct FunctionSystem<F, P: SystemParam, M> {
    f: F,
    state: P::State,
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
        let params = P::fetch(world, &mut self.state);
        SystemParamFunction::run(&mut self.f, params);
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
        (self.f)(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Commands;

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
}

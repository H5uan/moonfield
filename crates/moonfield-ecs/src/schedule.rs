//! Schedules: labeled, ordered collections of systems, ported from Bevy's
//! `bevy_ecs::schedule` at the mechanism level.
//!
//! A [`Schedule`] owns a set of systems and runs them on the calling thread.
//! Systems run in registration order unless reordered with
//! [`IntoSystemConfigs::before`] / [`IntoSystemConfigs::after`], which declare
//! constraints against another system's label (its function/closure type name
//! by default). Constraints are resolved with a stable topological sort when
//! the schedule changes — there is deliberately no parallel executor and no
//! per-run DAG work, but the constraint model already matches what a future
//! dependency resolver would consume.
//!
//! Command semantics: [`World::apply_commands`] runs after **every** system,
//! so a system's [`Commands`](crate::Commands) are visible to every system
//! that runs after it in the same schedule run. At the end of a run the
//! world's change tick advances once, giving change detection its per-run
//! window.

use std::any::{TypeId, type_name};
use std::collections::HashMap;
use std::marker::PhantomData;

use crate::{IntoSystem, System, World};

/// Marker for schedule labels: unit structs identifying a schedule.
///
/// Labels are defined by the app layer (e.g. `Startup` / `Update` / `Render`);
/// a label's identity is its `TypeId`, so labels are zero-sized and compared
/// statically.
pub trait ScheduleLabel: Send + Sync + 'static {}

/// One registered system plus its ordering constraints.
pub struct SystemConfig {
    system: Box<dyn System>,
    /// The label ordering constraints refer to. Defaults to the system's
    /// type name; overridable via [`IntoSystemConfigs::named`].
    label: String,
    before: Vec<String>,
    after: Vec<String>,
    /// The set this config belongs to ([`IntoSystemConfigs::in_set`]);
    /// `add_systems` expands it into anchor constraints.
    in_set: Option<&'static str>,
}

/// The result of chaining ordering constraints off a system (or tuple of
/// systems); registerable into a [`Schedule`] like any system.
pub struct SystemConfigs {
    configs: Vec<SystemConfig>,
}

/// Types that can be registered into a [`Schedule`]: a single system, a
/// [`SystemConfigs`] chain, or a tuple of either.
///
/// The chain methods apply to every system in the set (for a single system
/// that is just the system itself), mirroring Bevy's `IntoSystemConfigs`.
pub trait IntoSystemConfigs<M>: Sized {
    /// Convert into individual system registrations.
    fn into_configs(self) -> Vec<SystemConfig>;

    /// Override the label ordering constraints use to refer to this system.
    fn named(self, label: &'static str) -> SystemConfigs {
        let mut configs = self.into_configs();
        for config in &mut configs {
            config.label = label.to_string();
        }
        SystemConfigs { configs }
    }

    /// Run this system before `system` (referenced by its type name).
    fn before<S: 'static>(self, _system: &S) -> SystemConfigs {
        self.before_label(type_name::<S>())
    }

    /// Run this system after `system` (referenced by its type name).
    fn after<S: 'static>(self, _system: &S) -> SystemConfigs {
        self.after_label(type_name::<S>())
    }

    /// Run this system before the system registered with `label`.
    fn before_label(self, label: &'static str) -> SystemConfigs {
        let mut configs = self.into_configs();
        for config in &mut configs {
            config.before.push(label.to_string());
        }
        SystemConfigs { configs }
    }

    /// Run this system after the system registered with `label`.
    fn after_label(self, label: &'static str) -> SystemConfigs {
        let mut configs = self.into_configs();
        for config in &mut configs {
            config.after.push(label.to_string());
        }
        SystemConfigs { configs }
    }

    /// Run this system before the set registered with
    /// [`Schedule::add_sets`].
    fn before_set<S: SystemSet>(self) -> SystemConfigs {
        self.before_label(type_name::<S>())
    }

    /// Run this system after the set registered with
    /// [`Schedule::add_sets`].
    fn after_set<S: SystemSet>(self) -> SystemConfigs {
        self.after_label(type_name::<S>())
    }

    /// Run this system inside the set `S`: after the set's anchor and
    /// before the next anchor in the set chain, so systems of neighboring
    /// sets stay ordered without explicit constraints. The schedule resolves
    /// the next anchor at registration time.
    fn in_set<S: SystemSet>(self) -> SystemConfigs {
        let mut configs = self.into_configs();
        for config in &mut configs {
            config.in_set = Some(type_name::<S>());
        }
        SystemConfigs { configs }
    }
}

/// Marker for single systems registered into a schedule.
pub struct SingleSystemMarker<M>(PhantomData<fn() -> M>);

/// A named ordering anchor in a schedule: any unit struct identifying the
/// set. The anchor is a no-op system registered by [`Schedule::add_sets`];
/// systems attach to it with [`IntoSystemConfigs::before_set`] /
/// [`IntoSystemConfigs::after_set`].
pub trait SystemSet: Send + Sync + 'static {}

/// The no-op anchor system behind every set.
struct NopSystem;

impl System for NopSystem {
    fn name(&self) -> &str {
        "set anchor"
    }

    fn run(&mut self, _world: &mut World) {}
}

/// Types passable to [`Schedule::add_sets`]: one [`SystemSet`], or a tuple
/// of sets in chain order.
pub trait SetChain {
    /// The set labels in chain order.
    fn labels() -> Vec<&'static str>;
}

impl<S: SystemSet> SetChain for S {
    fn labels() -> Vec<&'static str> {
        vec![type_name::<S>()]
    }
}

macro_rules! impl_set_chain_tuple {
    ($($set:ident),*) => {
        impl<$($set: SystemSet),*> SetChain for ($($set,)*) {
            fn labels() -> Vec<&'static str> {
                vec![$(type_name::<$set>()),*]
            }
        }
    };
}

smaller_tuples_too!(impl_set_chain_tuple, S0, S1, S2, S3, S4, S5, S6, S7);

/// Marker for [`SystemConfigs`] chains registered into a schedule.
pub struct ChainedConfigsMarker;

/// Marker for tuples of systems registered into a schedule.
pub struct TupleConfigsMarker;

impl<S, M> IntoSystemConfigs<SingleSystemMarker<M>> for S
where
    S: IntoSystem<M>,
{
    fn into_configs(self) -> Vec<SystemConfig> {
        let system = self.into_system();
        let label = system.name().to_string();
        vec![SystemConfig {
            system,
            label,
            before: Vec::new(),
            after: Vec::new(),
            in_set: None,
        }]
    }
}

impl IntoSystemConfigs<ChainedConfigsMarker> for SystemConfigs {
    fn into_configs(self) -> Vec<SystemConfig> {
        self.configs
    }
}

// `smaller_tuples_too` expands a flat ident list, but each tuple element needs
// a paired (system, marker) type parameter, so tuples get their own recursive
// macro. Arity 0-8, matching the `SystemParam` tuples.
macro_rules! impl_configs_tuples {
    () => {
        impl IntoSystemConfigs<(TupleConfigsMarker,)> for () {
            fn into_configs(self) -> Vec<SystemConfig> {
                Vec::new()
            }
        }
    };
    (($name:ident, $mark:ident) $(, ($rest_name:ident, $rest_mark:ident))*) => {
        #[allow(non_snake_case)]
        impl<$name, $mark, $($rest_name, $rest_mark),*>
            IntoSystemConfigs<(TupleConfigsMarker, $mark, $($rest_mark,)*)>
            for ($name, $($rest_name,)*)
        where
            $name: IntoSystemConfigs<$mark>,
            $($rest_name: IntoSystemConfigs<$rest_mark>,)*
        {
            fn into_configs(self) -> Vec<SystemConfig> {
                let ($name, $($rest_name,)*) = self;
                #[allow(unused_mut)]
                let mut out = $name.into_configs();
                $(out.extend($rest_name.into_configs());)*
                out
            }
        }
        impl_configs_tuples!{$(($rest_name, $rest_mark)),*}
    };
}

impl_configs_tuples! {
    (A, MA), (B, MB), (C, MC), (D, MD), (E, ME), (F, MF), (G, MG), (H, MH)
}

/// An ordered collection of systems that runs on the calling thread.
///
/// Execution order is registration order, adjusted by `before`/`after`
/// constraints (resolved with a stable topological sort when the schedule
/// changes). Constraints referencing labels with no registered system are
/// ignored — they may point at systems living in other schedules. Cycles
/// panic.
#[derive(Default)]
pub struct Schedule {
    systems: Vec<SystemConfig>,
    /// Indices into `systems` in execution order; rebuilt when `dirty`.
    order: Vec<usize>,
    dirty: bool,
    /// The set anchors in chain order ([`Schedule::add_sets`]), used to
    /// expand `in_set` membership into anchor constraints.
    set_chain: Vec<String>,
}

impl Schedule {
    /// Create an empty schedule.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one or more systems (a system, a `.before()`/`.after()`
    /// chain, or a tuple of either). A config marked with
    /// [`IntoSystemConfigs::in_set`] gains anchor constraints: after its
    /// set's anchor and before the next anchor in the set chain.
    pub fn add_systems<M>(&mut self, systems: impl IntoSystemConfigs<M>) -> &mut Self {
        for mut config in systems.into_configs() {
            if let Some(set) = config.in_set {
                config.after.push(set.to_string());
                if let Some(next) = self.next_set_anchor(set) {
                    config.before.push(next.to_string());
                }
            }
            self.systems.push(config);
        }
        self.dirty = true;
        self
    }

    /// Register an ordered chain of [`SystemSet`]s: one no-op anchor system
    /// per set, each ordered after the previous. Pass a single set or a
    /// tuple in chain order — `add_sets((First, Second, Third))`. Systems
    /// attach with [`IntoSystemConfigs::in_set`] (membership, anchored on
    /// both sides) or `before_set` / `after_set` (one-sided).
    pub fn add_sets<S: SetChain>(&mut self, _sets: S) -> &mut Self {
        let mut previous: Option<String> = None;
        for label in S::labels() {
            let mut config = SystemConfig {
                system: Box::new(NopSystem),
                label: label.to_string(),
                before: Vec::new(),
                after: Vec::new(),
                in_set: None,
            };
            if let Some(previous) = previous.as_deref() {
                config.after.push(previous.to_string());
            }
            previous = Some(label.to_string());
            self.set_chain.push(label.to_string());
            self.systems.push(config);
        }
        self.dirty = true;
        self
    }

    /// The anchor following `set` in the set chain, if any.
    fn next_set_anchor(&self, set: &str) -> Option<String> {
        let index = self.set_chain.iter().position(|label| label == set)?;
        self.set_chain.get(index + 1).cloned()
    }

    /// The number of registered systems.
    pub fn len(&self) -> usize {
        self.systems.len()
    }

    /// Whether no systems are registered.
    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }

    /// Run every system once, in resolved order, applying deferred commands
    /// after each system and advancing the world's change tick at the end.
    pub fn run(&mut self, world: &mut World) {
        if self.dirty {
            self.rebuild_order();
        }
        let order = std::mem::take(&mut self.order);
        for &index in &order {
            self.systems[index].system.run(world);
            world.apply_commands();
        }
        self.order = order;
        world.increment_change_tick();
    }

    /// Resolve `before`/`after` constraints into an execution order: a stable
    /// topological sort where registration order breaks ties.
    fn rebuild_order(&mut self) {
        let n = self.systems.len();
        let mut by_label: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, slot) in self.systems.iter().enumerate() {
            by_label.entry(slot.label.as_str()).or_default().push(i);
        }

        // successor edges + in-degree, deduplicated via a set of pairs.
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (i, slot) in self.systems.iter().enumerate() {
            for target in &slot.after {
                for &j in by_label.get(target.as_str()).into_iter().flatten() {
                    edges.push((j, i));
                }
            }
            for target in &slot.before {
                for &j in by_label.get(target.as_str()).into_iter().flatten() {
                    edges.push((i, j));
                }
            }
        }
        edges.sort_unstable();
        edges.dedup();

        let mut successors: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_degree = vec![0usize; n];
        for &(from, to) in &edges {
            if from == to {
                panic!(
                    "system `{}` has an ordering constraint on itself",
                    self.systems[from].label
                );
            }
            successors[from].push(to);
            in_degree[to] += 1;
        }

        // Kahn's algorithm; the ready set is kept sorted by registration index
        // so unconstrained systems keep their registration order.
        let mut ready: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(&next) = ready.first() {
            ready.remove(0);
            order.push(next);
            for &succ in &successors[next] {
                in_degree[succ] -= 1;
                if in_degree[succ] == 0 {
                    let pos = ready.binary_search(&succ).unwrap_err();
                    ready.insert(pos, succ);
                }
            }
        }

        if order.len() != n {
            let stuck: Vec<&str> = (0..n)
                .filter(|&i| in_degree[i] > 0)
                .map(|i| self.systems[i].label.as_str())
                .collect();
            panic!("system ordering cycle involving: {}", stuck.join(", "));
        }

        self.order = order;
        self.dirty = false;
    }
}

/// The world's schedule storage, held as a resource so schedules are world
/// data: [`World::add_systems`](crate::World::add_systems) registers into it
/// and [`World::run_schedule`](crate::World::run_schedule) is the single run
/// primitive — a system running inside a schedule can run other schedules.
#[derive(Default)]
pub struct Schedules(HashMap<TypeId, Schedule>);

impl Schedules {
    /// The schedule registered under `label`, if any.
    pub fn get(&self, label: &TypeId) -> Option<&Schedule> {
        self.0.get(label)
    }

    /// The schedule registered under `label`, inserting an empty one on a miss.
    pub(crate) fn entry(&mut self, label: TypeId) -> &mut Schedule {
        self.0.entry(label).or_default()
    }

    /// Remove the schedule registered under `label`, if any.
    pub(crate) fn remove(&mut self, label: &TypeId) -> Option<Schedule> {
        self.0.remove(label)
    }

    /// Put `schedule` back under `label`.
    pub(crate) fn insert(&mut self, label: TypeId, schedule: Schedule) {
        self.0.insert(label, schedule);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commands, Local, Query, ResMut};

    #[derive(Debug, Clone, PartialEq)]
    struct Pos(f32);

    #[derive(Debug, Default)]
    struct Log(Vec<&'static str>);

    #[derive(Debug, Default)]
    struct Count(u32);

    fn first(mut log: ResMut<Log>) {
        log.0.push("first");
    }

    fn second(mut log: ResMut<Log>) {
        log.0.push("second");
    }

    fn third(mut log: ResMut<Log>) {
        log.0.push("third");
    }

    fn run_schedule(schedule: &mut Schedule, world: &mut World) {
        schedule.run(world);
    }

    #[test]
    fn test_registration_order_is_default() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_systems((first, second, third));
        schedule.run(&mut world);
        assert_eq!(
            world.get_resource::<Log>().unwrap().0,
            ["first", "second", "third"]
        );
    }

    #[test]
    fn test_ordering_constraints_are_honored() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        // Registered in scramble order; constraints must win, and ties keep
        // registration order.
        schedule.add_systems((third.after(&second), second.after(&first), first));
        schedule.run(&mut world);
        assert_eq!(
            world.get_resource::<Log>().unwrap().0,
            ["first", "second", "third"]
        );
    }

    #[test]
    fn test_before_constraint() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_systems((second, first.before(&second)));
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["first", "second"]);
    }

    #[test]
    fn test_named_labels_and_label_constraints() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_systems((
            third.named("c"),
            first.after_label("c"),
            second.after_label("c"),
        ));
        schedule.run(&mut world);
        assert_eq!(
            world.get_resource::<Log>().unwrap().0,
            ["third", "first", "second"]
        );
    }

    #[test]
    #[should_panic(expected = "ordering cycle")]
    fn test_ordering_cycle_panics() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_systems((first.after(&second), second.after(&first)));
        schedule.run(&mut world);
    }

    #[test]
    fn test_constraint_to_unknown_label_is_ignored() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_systems((first.after_label("elsewhere"), second));
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["first", "second"]);
    }

    #[test]
    fn test_commands_apply_after_each_system() {
        fn spawner(commands: Commands) {
            commands.spawn((Pos(1.0),));
        }

        fn observer(query: Query<&Pos>, mut log: ResMut<Log>) {
            // Runs after `spawner` in the same schedule run and must observe
            // its spawned entity.
            match query.iter().count() {
                0 => log.0.push("empty"),
                _ => log.0.push("spawned"),
            }
        }

        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_systems((spawner, observer));
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["spawned"]);
    }

    #[test]
    fn test_local_persists_across_schedule_runs() {
        fn counter(mut n: Local<u32>, mut count: ResMut<Count>) {
            *n += 1;
            count.0 = *n;
        }

        let mut world = World::new();
        world.insert_resource(Count::default());
        let mut schedule = Schedule::new();
        schedule.add_systems(counter);
        schedule.run(&mut world);
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Count>().unwrap().0, 2);
    }

    #[test]
    fn test_change_tick_advances_per_run() {
        let mut world = World::new();
        let mut schedule = Schedule::new();
        let before = world.change_tick();
        schedule.run(&mut world);
        assert_eq!(world.change_tick().get(), before.get() + 1);
    }

    struct A;
    struct B;
    impl ScheduleLabel for A {}
    impl ScheduleLabel for B {}

    #[test]
    fn test_world_stored_schedule_runs() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        world.add_systems(A, (first, second));
        world.run_schedule(A);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["first", "second"]);
    }

    #[test]
    fn test_run_missing_schedule_is_noop() {
        let mut world = World::new();
        // Neither the resource nor the schedule exists.
        world.run_schedule(A);
        assert!(!world.contains_resource::<Schedules>());
    }

    #[test]
    fn test_nested_run_schedule_from_exclusive_system() {
        fn driver(world: &mut World) {
            world.run_schedule(B);
        }

        let mut world = World::new();
        world.insert_resource(Log::default());
        world.add_systems(A, (first, driver));
        world.add_systems(B, second);
        world.run_schedule(A);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["first", "second"]);
        // B was reinserted and runs again on a direct call.
        world.run_schedule(B);
        assert_eq!(
            world.get_resource::<Log>().unwrap().0,
            ["first", "second", "second"]
        );
    }

    struct SetA;
    struct SetB;
    impl SystemSet for SetA {}
    impl SystemSet for SetB {}

    #[test]
    fn test_sets_order_attached_systems() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_sets((SetA, SetB));
        // Registered in reverse order; the set constraints must win.
        schedule.add_systems((second.after_set::<SetB>(), first.before_set::<SetB>()));
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["first", "second"]);
    }

    #[test]
    fn test_in_set_memberships_of_neighboring_sets_stay_ordered() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        let mut schedule = Schedule::new();
        schedule.add_sets((SetA, SetB));
        // Registered in reverse order with no constraints between them;
        // set membership alone must order first (in SetA) before second
        // (in SetB).
        schedule.add_systems((second.in_set::<SetB>(), first.in_set::<SetA>()));
        schedule.run(&mut world);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["first", "second"]);
    }

    #[test]
    fn test_same_label_rerun_inside_its_own_schedule_is_noop() {
        fn recursive(world: &mut World) {
            // The entry is out for the current run; this must not recurse.
            world.run_schedule(A);
            world.run_schedule(B);
        }

        let mut world = World::new();
        world.insert_resource(Log::default());
        world.add_systems(A, recursive);
        world.add_systems(B, second);
        world.run_schedule(A);
        assert_eq!(world.get_resource::<Log>().unwrap().0, ["second"]);
    }
}

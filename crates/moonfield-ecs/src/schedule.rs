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

use std::any::type_name;
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
}

/// Marker for single systems registered into a schedule.
pub struct SingleSystemMarker<M>(PhantomData<fn() -> M>);

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
}

impl Schedule {
    /// Create an empty schedule.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one or more systems (a system, a `.before()`/`.after()`
    /// chain, or a tuple of either).
    pub fn add_systems<M>(&mut self, systems: impl IntoSystemConfigs<M>) -> &mut Self {
        self.systems.extend(systems.into_configs());
        self.dirty = true;
        self
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
}

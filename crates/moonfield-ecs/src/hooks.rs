//! Component lifecycle hooks, ported from Bevy's `ComponentHooks`
//! (`bevy_ecs::lifecycle`).
//!
//! Hooks are per-component-type callbacks that run on structural changes —
//! they exist for structural side effects (keeping a hierarchy in sync,
//! maintaining an index, cleaning up resources), not for general-purpose
//! logic. Because [`Component`](crate::Component) stays a blanket impl (no
//! derives in this workspace), hooks are registered imperatively on the world:
//!
//! ```ignore
//! world.register_component_hooks::<Parent>().on_insert(|world, entity| {
//!     // update the new parent's Children list...
//! });
//! ```
//!
//! # Firing points
//!
//! - **spawn** (`spawn`, `spawn_at`, [`Commands`](crate::Commands) spawns):
//!   `on_add` then `on_insert` for each component.
//! - **insert of a new component** (`insert_component`, `insert_bundle`):
//!   `on_add` then `on_insert`.
//! - **insert replacing an existing component**: `on_discard` (old value still
//!   in place) then `on_insert`.
//! - **remove**: `on_discard` (value still in place) then `on_remove` (value
//!   gone).
//! - **despawn**: `on_despawn` first (every component still in place — this is
//!   where linked-spawn cleanup runs), then `on_discard`, then `on_remove`,
//!   per component.
//!
//! `on_despawn`/`on_discard` run *before* the structural change and
//! `on_add`/`on_insert`/`on_remove` run *after* it, so hooks always execute
//! with the world in a structurally consistent state and receive full
//! `&mut World` access. A hook may freely mutate *other* entities (that is
//! the relationships use case); mutating the hooked entity's own structure
//! from a discard hook aborts the pending operation gracefully (it re-resolves
//! the entity afterwards).
//!
//! # Limitations (minimal port)
//!
//! - While a hook is running it is temporarily taken out of the registry, so
//!   the same hook never fires recursively (a hook for `T` inserting `T` on
//!   another entity does not re-enter itself). Hooks on *other* components
//!   fire normally, so nested hook chains work.
//! - `spawn_batch` and `World::clear` do **not** fire hooks (cold bulk paths).
//! - Bevy's `RelationshipHookMode` (skip/run-if-not-linked) is not ported; the
//!   despawn-before-discard firing order covers the linked-spawn case it
//!   exists for.

use std::any::TypeId;

use crate::{Entity, World};

/// A component lifecycle hook, run with full world access and the entity the
/// component event happened to.
pub type ComponentHook = Box<dyn FnMut(&mut World, Entity)>;

/// The lifecycle event kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HookKind {
    Add,
    Insert,
    Discard,
    Remove,
    Despawn,
}

/// Lifecycle hooks registered for one component type.
///
/// Obtained via [`World::register_component_hooks`]; each registration
/// replaces the previous hook of that kind (if any).
#[derive(Default)]
pub struct ComponentHooks {
    pub(crate) on_add: Option<ComponentHook>,
    pub(crate) on_insert: Option<ComponentHook>,
    pub(crate) on_discard: Option<ComponentHook>,
    pub(crate) on_remove: Option<ComponentHook>,
    pub(crate) on_despawn: Option<ComponentHook>,
}

impl ComponentHooks {
    /// Register a hook run when this component is added to an entity, after
    /// the add. Always runs before `on_insert`. Spawning counts as adding all
    /// of the entity's components.
    pub fn on_add(&mut self, hook: impl FnMut(&mut World, Entity) + 'static) -> &mut Self {
        self.on_add = Some(Box::new(hook));
        self
    }

    /// Register a hook run when this component is inserted — on add (after
    /// `on_add`) and on replace (after `on_discard`), once the new value is in
    /// place.
    pub fn on_insert(&mut self, hook: impl FnMut(&mut World, Entity) + 'static) -> &mut Self {
        self.on_insert = Some(Box::new(hook));
        self
    }

    /// Register a hook run when this component is about to be dropped — on
    /// replace and on remove — *before* the old value disappears, so the hook
    /// can still read it.
    pub fn on_discard(&mut self, hook: impl FnMut(&mut World, Entity) + 'static) -> &mut Self {
        self.on_discard = Some(Box::new(hook));
        self
    }

    /// Register a hook run when this component is removed from an entity
    /// (including on despawn), after the value is gone.
    pub fn on_remove(&mut self, hook: impl FnMut(&mut World, Entity) + 'static) -> &mut Self {
        self.on_remove = Some(Box::new(hook));
        self
    }

    /// Register a hook run when the entity holding this component is
    /// despawned — before any `on_discard` hooks, while every component is
    /// still in place. This is the hook linked-spawn relationship targets use
    /// to despawn their sources.
    pub fn on_despawn(&mut self, hook: impl FnMut(&mut World, Entity) + 'static) -> &mut Self {
        self.on_despawn = Some(Box::new(hook));
        self
    }

    /// Take a hook out of the registry while it runs (recursion guard).
    pub(crate) fn take(&mut self, kind: HookKind) -> Option<ComponentHook> {
        match kind {
            HookKind::Add => self.on_add.take(),
            HookKind::Insert => self.on_insert.take(),
            HookKind::Discard => self.on_discard.take(),
            HookKind::Remove => self.on_remove.take(),
            HookKind::Despawn => self.on_despawn.take(),
        }
    }

    /// Put a hook back after it ran, unless a new one was registered in the
    /// meantime.
    pub(crate) fn restore(&mut self, kind: HookKind, hook: ComponentHook) {
        let slot = match kind {
            HookKind::Add => &mut self.on_add,
            HookKind::Insert => &mut self.on_insert,
            HookKind::Discard => &mut self.on_discard,
            HookKind::Remove => &mut self.on_remove,
            HookKind::Despawn => &mut self.on_despawn,
        };
        if slot.is_none() {
            *slot = Some(hook);
        }
    }
}

impl World {
    /// Register lifecycle hooks for component `T`, replacing any hooks of the
    /// same kinds previously registered.
    ///
    /// Panics may follow if entities with `T` already exist and the new hooks
    /// assume they observed every historical add — register hooks before
    /// spawning (same guidance as Bevy).
    pub fn register_component_hooks<T: crate::Component>(&mut self) -> &mut ComponentHooks {
        self.component_hooks.entry(TypeId::of::<T>()).or_default()
    }

    /// Run one hook, if registered. The hook is taken out of the registry
    /// while it runs so it cannot fire recursively, then restored.
    pub(crate) fn fire_hook(&mut self, kind: HookKind, component: TypeId, entity: Entity) {
        let Some(mut hook) = self
            .component_hooks
            .get_mut(&component)
            .and_then(|hooks| hooks.take(kind))
        else {
            return;
        };
        hook(self, entity);
        if let Some(hooks) = self.component_hooks.get_mut(&component) {
            hooks.restore(kind, hook);
        }
    }

    /// Fire the `on_add` → `on_insert` pair for a freshly added component.
    pub(crate) fn fire_component_added(&mut self, component: TypeId, entity: Entity) {
        self.fire_hook(HookKind::Add, component, entity);
        self.fire_hook(HookKind::Insert, component, entity);
    }
}

#[cfg(test)]
mod tests {
    use crate::{Component, Entity, World};

    #[derive(Debug, PartialEq)]
    struct A(u32);
    #[derive(Debug, PartialEq)]
    struct B(u32);

    #[derive(Default, Debug, PartialEq)]
    struct Log(Vec<(&'static str, &'static str)>);

    fn record(world: &mut World, component: &'static str, event: &'static str) {
        let Some(mut log) = world.get_resource_mut::<Log>() else {
            return;
        };
        log.0.push((component, event));
    }

    fn register_all<T: Component>(world: &mut World, name: &'static str) {
        world
            .register_component_hooks::<T>()
            .on_add(move |world, _| record(world, name, "add"))
            .on_insert(move |world, _| record(world, name, "insert"))
            .on_discard(move |world, _| record(world, name, "discard"))
            .on_remove(move |world, _| record(world, name, "remove"));
    }

    fn take_log(world: &mut World) -> Vec<(&'static str, &'static str)> {
        std::mem::take(&mut world.get_resource_mut::<Log>().unwrap().0)
    }

    #[test]
    fn test_spawn_fires_add_then_insert() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        register_all::<A>(&mut world, "A");

        world.spawn((A(1),));
        assert_eq!(take_log(&mut world), [("A", "add"), ("A", "insert")]);
    }

    #[test]
    fn test_insert_new_component_fires_add_and_insert() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        register_all::<B>(&mut world, "B");

        let e = world.spawn((A(1),));
        world.insert_component(e, B(2));
        assert_eq!(take_log(&mut world), [("B", "add"), ("B", "insert")]);
    }

    #[test]
    fn test_replace_fires_discard_then_insert_with_values() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        register_all::<A>(&mut world, "A");

        let e = world.spawn((A(1),));
        take_log(&mut world); // drain spawn's add+insert

        // Discard sees the old value; insert sees the new one. (These replace
        // the hooks registered above.)
        world
            .register_component_hooks::<A>()
            .on_discard(|world, entity| {
                let old = world.get_component::<A>(entity).unwrap().0;
                assert_eq!(old, 1);
                record(world, "A", "old");
            });
        world
            .register_component_hooks::<A>()
            .on_insert(|world, entity| {
                let new = world.get_component::<A>(entity).unwrap().0;
                assert_eq!(new, 2);
                record(world, "A", "new");
            });

        world.insert_component(e, A(2));
        let log = take_log(&mut world);
        assert_eq!(log, [("A", "old"), ("A", "new")]);
        // Replacing must not fire on_add.
        assert!(!log.contains(&("A", "add")));
    }

    #[test]
    fn test_remove_fires_discard_then_remove() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        register_all::<A>(&mut world, "A");

        let e = world.spawn((A(1), B(2)));
        take_log(&mut world);
        let removed = world.remove_component::<A>(e);
        assert_eq!(removed, Some(A(1)));
        assert_eq!(take_log(&mut world), [("A", "discard"), ("A", "remove")]);
        // B was untouched.
        assert_eq!(world.get_component::<B>(e), Some(&B(2)));
    }

    #[test]
    fn test_despawn_fires_discard_and_remove_per_component() {
        let mut world = World::new();
        world.insert_resource(Log::default());
        register_all::<A>(&mut world, "A");
        register_all::<B>(&mut world, "B");

        let e = world.spawn((A(1), B(2)));
        take_log(&mut world);
        world.despawn(e).unwrap();

        let log = take_log(&mut world);
        assert!(log.contains(&("A", "discard")));
        assert!(log.contains(&("A", "remove")));
        assert!(log.contains(&("B", "discard")));
        assert!(log.contains(&("B", "remove")));
        // Discard always precedes its remove.
        let pos_a_discard = log.iter().position(|&e| e == ("A", "discard")).unwrap();
        let pos_a_remove = log.iter().position(|&e| e == ("A", "remove")).unwrap();
        assert!(pos_a_discard < pos_a_remove);
    }

    #[test]
    fn test_hook_can_mutate_other_entities() {
        // The relationships use case: a hook on one entity maintains state on
        // another.
        struct Target(Entity);
        struct Linked;

        let mut world = World::new();
        let other = world.spawn(());
        world.insert_resource(Target(other));

        world
            .register_component_hooks::<A>()
            .on_insert(|world, _entity| {
                let target = world.get_resource::<Target>().unwrap().0;
                world.insert_component(target, Linked);
            });

        let e = world.spawn((A(1),));
        assert!(world.get_component::<A>(e).is_some());
        assert!(world.get_component::<Linked>(other).is_some());
    }

    #[test]
    fn test_nested_hooks_fire_but_same_hook_does_not_recurse() {
        struct Count(u32);

        let mut world = World::new();
        world.insert_resource(Count(0));
        let other = world.spawn(());

        // Inserting A inserts B on the same entity (nested hook fires)...
        world.register_component_hooks::<B>().on_insert(|world, _| {
            world.get_resource_mut::<Count>().unwrap().0 += 1;
        });
        world
            .register_component_hooks::<A>()
            .on_insert(move |world, entity| {
                world.get_resource_mut::<Count>().unwrap().0 += 1;
                if entity != other {
                    // ...and inserting A on `other` from here must not re-enter
                    // this hook.
                    world.insert_component(other, A(9));
                }
            });

        let e = world.spawn((A(1),));
        world.insert_component(e, B(2));
        // A's insert fired once (for `e`), B's once; the nested A insert on
        // `other` did not re-enter A's hook.
        assert_eq!(world.get_resource::<Count>().unwrap().0, 2);
        assert_eq!(world.get_component::<A>(other), Some(&A(9)));
    }

    #[test]
    fn test_hooks_fire_through_commands_and_bundle_insert() {
        use crate::Commands;

        let mut world = World::new();
        world.insert_resource(Log::default());
        register_all::<A>(&mut world, "A");
        register_all::<B>(&mut world, "B");

        let e = world.spawn(());
        {
            let commands = Commands::new(&world);
            commands.entity(e).insert((A(1), B(2)));
        }
        world.apply_commands();

        let log = take_log(&mut world);
        // Hooks fire per component in the bundle's sorted type order; each
        // component's add precedes its insert.
        for component in ["A", "B"] {
            let add = log.iter().position(|&e| e == (component, "add")).unwrap();
            let insert = log
                .iter()
                .position(|&e| e == (component, "insert"))
                .unwrap();
            assert!(add < insert);
        }
        assert_eq!(log.len(), 4);
    }
}

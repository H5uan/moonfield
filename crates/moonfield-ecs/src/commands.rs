use crate::{Component, Entity, EntityChanges, World};

/// Deferred command buffer for structural world changes.
///
/// Commands are **not** applied immediately; call [`World::apply_commands`]
/// (or the equivalent inside a system runner) to flush them.
pub struct Commands<'a> {
    changes: &'a mut EntityChanges,
}

impl<'a> Commands<'a> {
    pub(crate) fn new(changes: &'a mut EntityChanges) -> Self {
        Self { changes }
    }

    /// Queue an entity to be spawned with a single component.
    pub fn spawn<C: Component + Clone>(&mut self, components: (C,)) {
        let c = components.0;
        let mut bundle: Vec<Box<dyn FnOnce(Entity, &mut World)>> = Vec::new();
        bundle.push(Box::new(move |e, world| {
            world.insert_component(e, c);
        }));
        self.changes.to_spawn.push(bundle);
    }

    /// Queue an entity to be spawned with two components.
    pub fn spawn2<C1: Component + Clone, C2: Component + Clone>(&mut self, c1: C1, c2: C2) {
        let mut bundle: Vec<Box<dyn FnOnce(Entity, &mut World)>> = Vec::new();
        bundle.push(Box::new(move |e, world| {
            world.insert_component(e, c1);
        }));
        bundle.push(Box::new(move |e, world| {
            world.insert_component(e, c2);
        }));
        self.changes.to_spawn.push(bundle);
    }

    /// Queue an entity for despawn.
    pub fn despawn(&mut self, entity: Entity) {
        self.changes.to_despawn.push(entity);
    }
}

/// Owned command queue that can be applied to a world later.
#[derive(Default)]
pub struct CommandQueue {
    changes: EntityChanges,
}

impl CommandQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn<C: Component + Clone>(&mut self, components: (C,)) {
        let c = components.0;
        let mut bundle: Vec<Box<dyn FnOnce(Entity, &mut World)>> = Vec::new();
        bundle.push(Box::new(move |e, world| {
            world.insert_component(e, c);
        }));
        self.changes.to_spawn.push(bundle);
    }

    pub fn despawn(&mut self, entity: Entity) {
        self.changes.to_despawn.push(entity);
    }

    pub fn apply(self, world: &mut World) {
        let mut changes = self.changes;
        changes.apply(world);
    }
}

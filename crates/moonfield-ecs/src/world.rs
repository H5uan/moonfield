use std::any::TypeId;
use std::collections::HashMap;

use crate::{
    component::{Component, ComponentStorage, ErasedStorage},
    entities::{Entity, EntityId},
    resource::Resource,
    Commands, EntityChanges, Query, Resources,
};

/// The central storage of an ECS application.
///
/// Holds all entities, component storages, and resources, and provides the main
/// API for spawning, querying, and mutating the simulation state.
#[derive(Default)]
pub struct World {
    entities: EntityId,
    components: HashMap<TypeId, ErasedStorage>,
    resources: Resources,
    changes: EntityChanges,
}

impl World {
    /// Create an empty world.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a new entity with no components.
    pub fn spawn_empty(&mut self) -> Entity {
        self.entities.alloc()
    }

    /// Spawn an entity with a single component.
    pub fn spawn<C: Component>(&mut self, components: (C,)) -> Entity {
        let e = self.entities.alloc();
        self.insert_component(e, components.0);
        e
    }

    /// Spawn an entity with two components.
    pub fn spawn2<C1: Component, C2: Component>(&mut self, c1: C1, c2: C2) -> Entity {
        let e = self.entities.alloc();
        self.insert_component(e, c1);
        self.insert_component(e, c2);
        e
    }

    /// Despawn an entity and all its components.
    ///
    /// Returns `true` if the entity existed.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.entities.free(entity) {
            return false;
        }
        for storage in self.components.values_mut() {
            storage.clear_entity(entity);
        }
        true
    }

    /// Insert a component for an existing entity (or replace if already present).
    pub fn insert_component<C: Component>(&mut self, entity: Entity, component: C) {
        let type_id = TypeId::of::<C>();
        let storage = self
            .components
            .entry(type_id)
            .or_insert_with(|| ErasedStorage::new::<C>());
        storage
            .get_mut::<C>()
            .expect("type mismatch in component map")
            .insert(entity, component);
    }

    /// Remove a component from an entity, returning it if it existed.
    pub fn remove_component<C: Component>(&mut self, entity: Entity) -> Option<C> {
        self.components
            .get_mut(&TypeId::of::<C>())?
            .get_mut::<C>()?
            .remove(entity)
    }

    /// Get a reference to a component on an entity.
    pub fn get_component<C: Component>(&self, entity: Entity) -> Option<&C> {
        self.components
            .get(&TypeId::of::<C>())?
            .get::<C>()?
            .get(entity)
    }

    /// Get a mutable reference to a component on an entity.
    pub fn get_component_mut<C: Component>(&mut self, entity: Entity) -> Option<&mut C> {
        self.components
            .get_mut(&TypeId::of::<C>())?
            .get_mut::<C>()?
            .get_mut(entity)
    }

    /// Query the world for a combination of components.
    ///
    /// Supports `&T`, `&mut T`, tuples, `Option<&T>`, and `Entity`.
    pub fn query<'a, Q: Query + 'a>(&'a self) -> impl Iterator<Item = Q::Item<'a>> + 'a {
        Q::fetch(self).map(|(_, item)| item)
    }

    /// Query the world for a mutable combination of components.
    pub fn query_mut<'a, Q: Query + 'a>(&'a mut self) -> impl Iterator<Item = Q::Item<'a>> + 'a {
        Q::fetch_mut(self).map(|(_, item)| item)
    }

    // ------------------------------------------------------------------
    // Resources
    // ------------------------------------------------------------------

    /// Insert a resource into the world, replacing any existing one of the same type.
    pub fn insert_resource<R: Resource>(&mut self, res: R) {
        self.resources.insert(res);
    }

    /// Get an immutable reference to a resource.
    pub fn get_resource<R: Resource>(&self) -> Option<std::cell::Ref<'_, R>> {
        self.resources.get::<R>()
    }

    /// Get a mutable reference to a resource.
    pub fn get_resource_mut<R: Resource>(&self) -> Option<std::cell::RefMut<'_, R>> {
        self.resources.get_mut::<R>()
    }

    /// Remove a resource from the world, returning it if it existed.
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.resources.remove::<R>()
    }

    // ------------------------------------------------------------------
    // Commands
    // ------------------------------------------------------------------

    /// Obtain a [`Commands`] handle that can queue deferred structural changes.
    pub fn commands(&mut self) -> Commands<'_> {
        Commands::new(&mut self.changes)
    }

    /// Apply all queued commands (spawn / despawn) immediately.
    pub fn apply_commands(&mut self) {
        let mut changes = std::mem::take(&mut self.changes);
        changes.apply(self);
        self.changes = changes;
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    pub(crate) fn entities(&self) -> &EntityId {
        &self.entities
    }

    pub(crate) fn entities_mut(&mut self) -> &mut EntityId {
        &mut self.entities
    }

    pub(crate) fn component_storage<T: Component>(&self) -> Option<&ComponentStorage<T>> {
        self.components.get(&TypeId::of::<T>())?.get::<T>()
    }

    pub(crate) fn component_storage_mut<T: Component>(
        &mut self,
    ) -> Option<&mut ComponentStorage<T>> {
        self.components.get_mut(&TypeId::of::<T>())?.get_mut::<T>()
    }
}

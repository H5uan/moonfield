use std::any::Any;
use std::collections::HashMap;

use crate::Entity;

/// Marker trait for types that can be used as components.
///
/// Automatically implemented for all `Send + Sync + 'static` types.
pub trait Component: Send + Sync + 'static {}

/// Dense storage for components of a single type.
///
/// Uses a *sparse–dense* mapping: `entity_indices` maps entity id → dense index,
/// and `dense_data` holds the actual values packed together.
pub struct ComponentStorage<T: Component> {
    /// maps entity raw id -> optional dense index
    entity_indices: HashMap<u64, usize>,
    dense_data: Vec<T>,
    dense_entities: Vec<Entity>,
}

impl<T: Component> Default for ComponentStorage<T> {
    fn default() -> Self {
        Self {
            entity_indices: HashMap::new(),
            dense_data: Vec::new(),
            dense_entities: Vec::new(),
        }
    }
}

impl<T: Component> ComponentStorage<T> {
    pub fn insert(&mut self, entity: Entity, value: T) {
        let raw = entity.id();
        if let Some(&idx) = self.entity_indices.get(&raw) {
            self.dense_data[idx] = value;
        } else {
            let idx = self.dense_data.len();
            self.dense_data.push(value);
            self.dense_entities.push(entity);
            self.entity_indices.insert(raw, idx);
        }
    }

    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let raw = entity.id();
        let idx = self.entity_indices.remove(&raw)?;
        let last = self.dense_data.len() - 1;

        if idx != last {
            // swap-remove to keep the dense array contiguous
            self.dense_data.swap(idx, last);
            self.dense_entities.swap(idx, last);
            let moved_entity = self.dense_entities[idx];
            self.entity_indices.insert(moved_entity.id(), idx);
        }

        self.dense_entities.pop();
        self.dense_data.pop()
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        let idx = *self.entity_indices.get(&entity.id())?;
        self.dense_data.get(idx)
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        let idx = *self.entity_indices.get(&entity.id())?;
        self.dense_data.get_mut(idx)
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.entity_indices.contains_key(&entity.id())
    }

    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.dense_entities.iter().copied().zip(self.dense_data.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.dense_entities.iter().copied().zip(self.dense_data.iter_mut())
    }

    pub fn len(&self) -> usize {
        self.dense_data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dense_data.is_empty()
    }

    /// Clear all entries associated with a given entity. Used when an entity is despawned.
    pub fn clear_entity(&mut self, entity: Entity) {
        self.remove(entity);
    }
}

/// Type-erased wrapper so that different component types can live in the same map.
///
/// Note: `Box<dyn Any>` is used here instead of `Box<dyn Any + Send + Sync>` because
/// Rust's `Any::downcast_mut` does not work on `dyn Any + Send + Sync` trait objects.
/// Since `Component` requires `Send + Sync`, all concrete storages are soundly `Send + Sync`.
pub struct ErasedStorage {
    inner: Box<dyn Any>,
    remove_fn: Box<dyn Fn(&mut dyn Any, Entity) + Send + Sync>,
    clear_fn: Box<dyn Fn(&mut dyn Any, Entity) + Send + Sync>,
    len_fn: Box<dyn Fn(&dyn Any) -> usize + Send + Sync>,
}

impl Default for ErasedStorage {
    fn default() -> Self {
        Self {
            inner: Box::new(()),
            remove_fn: Box::new(|_, _| {}),
            clear_fn: Box::new(|_, _| {}),
            len_fn: Box::new(|_| 0),
        }
    }
}

impl ErasedStorage {
    pub fn new<T: Component>() -> Self {
        Self {
            inner: Box::new(ComponentStorage::<T>::default()),
            remove_fn: Box::new(|any: &mut dyn Any, e: Entity| {
                let storage = any.downcast_mut::<ComponentStorage<T>>().unwrap();
                storage.remove(e);
            }),
            clear_fn: Box::new(|any: &mut dyn Any, e: Entity| {
                let storage = any.downcast_mut::<ComponentStorage<T>>().unwrap();
                storage.clear_entity(e);
            }),
            len_fn: Box::new(|any: &dyn Any| {
                let storage = any.downcast_ref::<ComponentStorage<T>>().unwrap();
                storage.len()
            }),
        }
    }

    pub fn get<T: Component>(&self) -> Option<&ComponentStorage<T>> {
        self.inner.downcast_ref::<ComponentStorage<T>>()
    }

    pub fn get_mut<T: Component>(&mut self) -> Option<&mut ComponentStorage<T>> {
        self.inner.downcast_mut::<ComponentStorage<T>>()
    }

    pub fn remove(&mut self, entity: Entity) {
        let any = self.inner.as_mut();
        (self.remove_fn)(any, entity);
    }

    pub fn clear_entity(&mut self, entity: Entity) {
        let any = self.inner.as_mut();
        (self.clear_fn)(any, entity);
    }

    pub fn len(&self) -> usize {
        let any = self.inner.as_ref();
        (self.len_fn)(any)
    }
}

/// All `Send + Sync + 'static` types are components.
impl<T: Send + Sync + 'static> Component for T {}

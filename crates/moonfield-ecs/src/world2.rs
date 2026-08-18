use std::any::TypeId;
use std::borrow::Borrow;
use std::collections::{hash_map::Entry, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::AtomicU64;

use crate::archetype::{Archetype, ComponentMeta, TypeIdMap};
use crate::bundle::{Bundle, DynamicBundle};
use crate::entities::{AllocManyState, Entities, Location, NoSuchEntity, ReserveEntitiesIterator};
use crate::{Component, Entity, Query, Resources};
use std::mem;

struct ArchetypeSet {
    index: HashMap<Box<[TypeId]>, u32>,
    archetypes: Vec<Archetype>,
}

impl ArchetypeSet {
    fn new() -> Self {
        // `flush` assumes archetype 0 always exists, representing entities with no components.
        Self {
            index: Some((Box::default(), 0)).into_iter().collect(),
            archetypes: vec![Archetype::new(Vec::new())],
        }
    }

    /// Find the archetype ID for exactly this component set, creating it on a miss.
    fn get<T, F>(&mut self, components: T, component_metas: F) -> u32
    where
        T: Borrow<[TypeId]> + Into<Box<[TypeId]>>,
        F: FnOnce() -> Vec<ComponentMeta>,
    {
        if let Some(&archetype_id) = self.index.get(components.borrow()) {
            return archetype_id;
        }

        self.insert(components.into(), component_metas())
    }

    fn insert(&mut self, components: Box<[TypeId]>, component_metas: Vec<ComponentMeta>) -> u32 {
        let archetype_id = u32::try_from(self.archetypes.len()).expect("too many archetypes");
        assert_ne!(archetype_id, u32::MAX, "too many archetypes");

        let archetype = Archetype::new(component_metas);
        match self.index.entry(components) {
            Entry::Occupied(_) => panic!("inserted duplicate archetype"),
            Entry::Vacant(entry) => {
                self.archetypes.push(archetype);
                entry.insert(archetype_id);
                archetype_id
            }
        }
    }

    fn get_mut(&mut self, id: u32) -> &mut Archetype {
        &mut self.archetypes[id as usize]
    }

    /// Resolve (creating if necessary) the archetype for a bundle's component set.
    fn get_for<B: DynamicBundle>(&mut self, components: &B) -> u32 {
        components.with_ids(|ids| self.get(ids, || components.component_meta()))
    }
}

struct InsertTarget {
    replaced: Vec<ComponentMeta>,
    retained: Vec<ComponentMeta>,
    index: u32,
}

#[derive(Default)]
struct IndexTypeIdHasher(u64);

impl Hasher for IndexTypeIdHasher {
    fn write_u32(&mut self, index: u32) {
        self.0 ^= u64::from(index);
    }

    fn write_u64(&mut self, type_id: u64) {
        self.0 ^= type_id;
    }

    fn write(&mut self, _bytes: &[u8]) {
        unreachable!()
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
type IndexTypeIdMap<V> = HashMap<(u32, TypeId), V, BuildHasherDefault<IndexTypeIdHasher>>;

pub struct SpawnBatchIter<'a, I>
where
    I: Iterator,
    I::Item: Bundle,
{
    inner: I,
    entities: &'a mut Entities,
    archetype_id: u32,
    archetype: &'a mut Archetype,
}

impl<I> Drop for SpawnBatchIter<'_, I>
where
    I: Iterator,
    I::Item: Bundle,
{
    fn drop(&mut self) {
        for _ in self {}
    }
}

impl<I> Iterator for SpawnBatchIter<'_, I>
where
    I: Iterator,
    I::Item: Bundle,
{
    type Item = Entity;

    fn next(&mut self) -> Option<Entity> {
        let components = self.inner.next()?;
        let entity = self.entities.alloc();
        let index = unsafe { self.archetype.allocate(entity.id) };
        unsafe {
            components.put(|ptr, component_meta| {
                self.archetype.put_ptr(
                    ptr,
                    *component_meta.id(),
                    component_meta.layout().size(),
                    index,
                );
            });
        }
        self.entities.meta[entity.id as usize].location = Location {
            archetype: self.archetype_id,
            index,
        };
        Some(entity)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<I, T> ExactSizeIterator for SpawnBatchIter<'_, I>
where
    I: ExactSizeIterator<Item = T>,
    T: Bundle,
{
    fn len(&self) -> usize {
        self.inner.len()
    }
}

pub struct SpawnColumnBatchIter<'a> {
    pending_end: usize,
    id_alloc: AllocManyState,
    entities: &'a mut Entities,
}

impl Iterator for SpawnColumnBatchIter<'_> {
    type Item = Entity;

    fn next(&mut self) -> Option<Entity> {
        let id = self.id_alloc.next(self.entities)?;
        Some(unsafe { self.entities.resolve_unknown_gen(id) })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len(), Some(self.len()))
    }
}

impl ExactSizeIterator for SpawnColumnBatchIter<'_> {
    fn len(&self) -> usize {
        self.id_alloc.len(self.entities)
    }
}

impl Drop for SpawnColumnBatchIter<'_> {
    fn drop(&mut self) {
        // Consume used freelist entries
        self.entities.finish_alloc_many(self.pending_end);
    }
}

pub struct World2 {
    entities: Entities,
    archetypes: ArchetypeSet,
    bundle_to_archetype: TypeIdMap<u32>,
    insert_edges: IndexTypeIdMap<InsertTarget>,
    remove_edges: IndexTypeIdMap<u32>,
    resources: Resources,

    id: AtomicU64,
}

impl Default for World2 {
    fn default() -> Self {
        Self::new()
    }
}

impl World2 {
    pub fn new() -> Self {
        Self {
            entities: Entities::default(),
            archetypes: ArchetypeSet::new(),
            bundle_to_archetype: TypeIdMap::default(),
            insert_edges: IndexTypeIdMap::default(),
            remove_edges: IndexTypeIdMap::default(),
            resources: Resources::default(),
            id: AtomicU64::new(0),
        }
    }

    pub fn flush(&mut self) {
        // zero is reserved for entities with no components.
        let archetype = self.archetypes.get_mut(0);
        self.entities
            .flush(|id, location| location.index = unsafe { archetype.allocate(id) });
    }

    /// Create an entity with certain components
    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
        self.flush();
        let entity = self.entities.alloc();
        self.spawn_inner(entity, components);
        entity
    }

    pub fn spawn_at(&mut self, entity: Entity, components: impl DynamicBundle) {
        self.flush();

        let loc = self.entities.alloc_at(entity);
        if let Some(loc) = loc {
            if let Some(moved) = unsafe {
                // It is possible that entity already exists in this location.
                // If so, we need to move it to the new location.
                // Otherwise, we can just insert it.
                self.archetypes
                    .get_mut(loc.archetype)
                    .remove(loc.index, true)
            } {
                self.entities.meta[moved as usize].location.index = loc.index;
            }
        }

        self.spawn_inner(entity, components);
    }

    fn spawn_inner(&mut self, entity: Entity, components: impl DynamicBundle) {
        let archetype_id = match components.key() {
            Some(k) => *self
                .bundle_to_archetype
                .entry(k)
                .or_insert_with(|| self.archetypes.get_for(&components)),
            None => self.archetypes.get_for(&components),
        };

        let index = unsafe {
            let archetype = self.archetypes.get_mut(archetype_id);
            let row = archetype.allocate(entity.id());
            components.put(|ptr, meta| {
                archetype.put_ptr(ptr, *meta.id(), meta.layout().size(), row);
            });
            row
        };

        self.entities.meta[entity.id as usize].location = Location {
            archetype: archetype_id,
            index,
        };
    }

    pub fn spawn_batch<I>(&mut self, iter: I) -> SpawnBatchIter<'_, I::IntoIter>
    where
        I: IntoIterator,
        I::Item: Bundle + 'static,
    {
        self.flush();
        let iter = iter.into_iter();
        let (lower, upper) = iter.size_hint();
        let archetype_id = self.reserve_inner::<I::Item>(
            u32::try_from(upper.unwrap_or(lower)).expect("iterator too large"),
        );
        SpawnBatchIter {
            inner: iter,
            entities: &mut self.entities,
            archetype_id,
            archetype: &mut self.archetypes.archetypes[archetype_id as usize],
        }
    }

    pub fn reserve_entities(&mut self, count: u32) -> ReserveEntitiesIterator<'_> {
        self.entities.reserve_entities(count)
    }

    pub fn reserve_entity(&self) -> Entity {
        self.entities.reserve_entity()
    }

    pub fn despawn(&mut self, entity: Entity) -> Result<(), NoSuchEntity> {
        self.flush();
        let loc = self.entities.free(entity)?;
        if let Some(moved) =
            unsafe { self.archetypes.archetypes[loc.archetype as usize].remove(loc.index, true) }
        {
            self.entities.meta[moved as usize].location.index = loc.index;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        for x in &mut self.archetypes.archetypes {
            x.clear();
        }
        self.entities.clear();
    }

    /// Whether `entity` still exists
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(entity)
    }

    pub fn reserve<T: Bundle + 'static>(&mut self, additional: u32) {
        self.reserve_inner::<T>(additional);
    }

    fn reserve_inner<T: Bundle + 'static>(&mut self, additional: u32) -> u32 {
        self.flush();
        self.entities.reserve(additional);
        let archetypes = &mut self.archetypes;
        let archetype_id = *self
            .bundle_to_archetype
            .entry(TypeId::of::<T>())
            .or_insert_with(|| {
                T::with_static_ids(|ids| {
                    archetypes.get(ids, || T::with_static_component_meta(|meta| meta.to_vec()))
                })
            });

        self.archetypes.archetypes[archetype_id as usize].reserve(additional);
        archetype_id
    }

    /// Spawn an entity with no components.
    ///
    /// The entity is placed in the empty archetype (archetype 0).
    pub fn spawn_empty(&mut self) -> Entity {
        self.flush();
        let entity = self.entities.alloc();
        let archetype = self.archetypes.get_mut(0);
        let index = unsafe { archetype.allocate(entity.id) };
        self.entities.meta[entity.id as usize].location = Location {
            archetype: 0,
            index,
        };
        entity
    }

    // ------------------------------------------------------------------
    // Resources
    // ------------------------------------------------------------------

    /// Insert a resource into the world, replacing any existing one of the same type.
    pub fn insert_resource<R: crate::Resource>(&mut self, res: R) {
        self.resources.insert(res);
    }

    /// Get an immutable reference to a resource.
    pub fn get_resource<R: crate::Resource>(&self) -> Option<std::cell::Ref<'_, R>> {
        self.resources.get::<R>()
    }

    /// Get a mutable reference to a resource.
    pub fn get_resource_mut<R: crate::Resource>(&self) -> Option<std::cell::RefMut<'_, R>> {
        self.resources.get_mut::<R>()
    }

    /// Remove a resource from the world, returning it if it existed.
    pub fn remove_resource<R: crate::Resource>(&mut self) -> Option<R> {
        self.resources.remove::<R>()
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Query the world for a combination of components.
    ///
    /// Yields `(Entity, item)` pairs where each item borrows from the world.
    pub fn query<'a, Q: Query>(&'a self) -> Q::Iter<'a> {
        Q::fetch(self)
    }

    /// Query the world for a mutable combination of components.
    pub fn query_mut<'a, Q: Query>(&'a mut self) -> Q::Iter<'a> {
        Q::fetch_mut(self)
    }

    // ------------------------------------------------------------------
    // Commands (no-op in the minimal archetype backend)
    // ------------------------------------------------------------------

    /// Deferred structural commands are not yet supported; `App` calls this
    /// unconditionally, so it is a safe no-op that keeps the app runner intact.
    pub fn apply_commands(&mut self) {}

    // ------------------------------------------------------------------
    // Internals (crate-visible for the archetype query engine)
    // ------------------------------------------------------------------

    pub(crate) fn raw_archetypes(&self) -> &[Archetype] {
        &self.archetypes.archetypes
    }

    pub(crate) fn raw_entity_meta(&self) -> &[crate::entities::EntityMeta] {
        &self.entities.meta
    }

    // ------------------------------------------------------------------
    // Component access by entity (entity → archetype row)
    // ------------------------------------------------------------------

    /// Get an immutable reference to `T` on `entity`, if present.
    pub fn get_component<T: Component>(&self, entity: Entity) -> Option<&T> {
        let loc = self.entities.get(entity).ok()?;
        let arch = &self.archetypes.archetypes[loc.archetype as usize];
        let col = arch.get_state::<T>()?;
        // SAFETY: we hold `&self`, so no `&mut` write to this column can be
        // outstanding; the row is within bounds relative to `self`.
        Some(unsafe { &*arch.get_base::<T>(col).as_ptr().add(loc.index as usize) })
    }

    /// Get a mutable reference to `T` on `entity`, if present.
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        // Resolve the location first so the borrow of `self.entities` is
        // released before we take a mutable borrow of the archetype slice.
        let loc = self.entities.get(entity).ok()?;
        let archetypes = &mut self.archetypes.archetypes;
        let arch = &mut archetypes[loc.archetype as usize];
        let col = arch.get_state::<T>()?;
        let base = unsafe { arch.get_base::<T>(col) };
        // SAFETY: `base` is derived from `arch` (mutably borrowed via `&mut self`);
        // the row is within bounds and no other `&mut T` can exist.
        Some(unsafe { &mut *base.as_ptr().add(loc.index as usize) })
    }

    /// Insert (or replace) component `T` on `entity`.
    ///
    /// If `entity` already has `T`, the value is written in place. Otherwise the
    /// entity is moved across archetypes into one whose type set is its current
    /// set plus `T`, preserving every already-present component.
    ///
    /// Returns `None` if `entity` does not exist.
    pub fn insert_component<T: Component>(&mut self, entity: Entity, value: T) -> Option<()> {
        self.flush();
        let loc = self.entities.get(entity).ok()?;
        let old_id = loc.archetype;
        let old_row = loc.index;
        let old_arch = &self.archetypes.archetypes[old_id as usize];

        // Replace in place when the archetype already holds `T`.
        if let Some(col) = old_arch.get_state::<T>() {
            // SAFETY: `&mut self` guarantees no aliasing `&T`/`&mut T` here.
            unsafe { *old_arch.get_base::<T>(col).as_ptr().add(old_row as usize) = value };
            return Some(());
        }

        // Build the target archetype type set = old set ∪ {T}, kept in the same
        // ordered form (alignment desc, then TypeId) used to key archetypes.
        let mut metas: Vec<ComponentMeta> = old_arch.component_metas().to_vec();
        metas.push(ComponentMeta::of::<T>());
        metas.sort_unstable();
        let ids: Box<[TypeId]> = metas.iter().map(|m| *m.id()).collect();
        let target_id = self.archetypes.get(ids, || metas.clone());
        assert_ne!(target_id, old_id, "archetype must gain a new type");

        // SAFETY: we refer to two distinct archetype slots by index; `get` above
        // may have reallocated `archetypes`, so the raw pointers are taken *after*
        // it. `old_id != target_id` makes the raw `&`/`&mut` disjoint.
        let old_raw: *const Archetype = &self.archetypes.archetypes[old_id as usize];
        let target_raw: *mut Archetype = &mut self.archetypes.archetypes[target_id as usize];

        // SAFETY: allocate a fresh row in the target archetype (no outstanding
        // borrows on its columns).
        let new_row = unsafe { (*target_raw).allocate(entity.id) };

        // Move every already-present component's bytes from the old row to the
        // new target row.
        // SAFETY: both archetypes are live; old_row/new_row are in bounds; we are
        // copying from a distinct allocation into the freshly allocated target row.
        unsafe {
            let old = &*old_raw;
            for meta in old.component_metas() {
                let size = meta.layout().size();
                if let Some(src) = old.get_ptr(*meta.id(), size, old_row) {
                    (*target_raw).put_ptr(src.as_ptr(), *meta.id(), size, new_row);
                }
            }
            // Write the newly inserted component value.
            let mut value = value;
            (*target_raw).put_ptr(
                (&mut value as *mut T).cast(),
                TypeId::of::<T>(),
                mem::size_of::<T>(),
                new_row,
            );
            mem::forget(value);
        }

        // Update the entity's location to the target archetype.
        self.entities.meta[entity.id as usize].location = Location {
            archetype: target_id,
            index: new_row,
        };

        // Remove the entity from its old archetype (swap-remove, no drop: its
        // component bytes were moved to the target archetype). Fix up the moved
        // entity's row index. The removed bytes are overwritten, not dropped, so
        // there is no double-drop.
        // SAFETY: no outstanding borrows on the old archetype's columns after the
        // copy above.
        if let Some(moved) =
            unsafe { self.archetypes.archetypes[old_id as usize].remove(old_row, false) }
        {
            self.entities.meta[moved as usize].location.index = old_row;
        }

        Some(())
    }
}

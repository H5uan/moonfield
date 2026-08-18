use std::any::TypeId;
use std::borrow::Borrow;
use std::collections::{hash_map::Entry, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::AtomicU64;

use crate::archetype::{Archetype, ComponentMeta, TypeIdMap};
use crate::bundle::{Bundle, DynamicBundle};
use crate::entities::{AllocManyState, Entities, Location, NoSuchEntity, ReserveEntitiesIterator};
use crate::{Entity, Query, Resources};

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
}

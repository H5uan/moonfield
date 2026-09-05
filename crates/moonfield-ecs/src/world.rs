use std::any::TypeId;
use std::borrow::Borrow;
use std::collections::{HashMap, hash_map::Entry};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::AtomicU64;

use crate::archetype::{Archetype, ComponentMeta, TypeIdMap};
use crate::bundle::{Bundle, DynamicBundle};
use crate::change_detection::{Mut, Ref, Tick};
use crate::commands::Command;
use crate::entities::{AllocManyState, Entities, Location, NoSuchEntity, ReserveEntitiesIterator};
use crate::hooks::{ComponentHooks, HookKind};
use crate::{Component, Entity, Resources, WorldQuery};
use std::cell::RefCell;
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
    /// The change tick recorded on every component written by this batch.
    tick: Tick,
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
        let tick = self.tick;
        unsafe {
            components.put(|ptr, component_meta| {
                self.archetype.put_ptr(
                    ptr,
                    *component_meta.id(),
                    component_meta.layout().size(),
                    index,
                );
                let column = self.archetype.get_state_by_id(component_meta.id()).unwrap();
                self.archetype.write_fresh_ticks(column, index, tick);
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

pub struct World {
    entities: Entities,
    archetypes: ArchetypeSet,
    bundle_to_archetype: TypeIdMap<u32>,
    insert_edges: IndexTypeIdMap<InsertTarget>,
    remove_edges: IndexTypeIdMap<u32>,
    resources: Resources,
    /// Deferred structural mutations queued by [`Commands`](crate::Commands),
    /// drained by [`Self::apply_commands`].
    command_queue: RefCell<Vec<Command>>,
    /// Lifecycle hooks registered per component type, keyed by [`TypeId`].
    pub(crate) component_hooks: HashMap<TypeId, ComponentHooks>,

    /// The world's current change-detection tick. Component writes record this
    /// value; it advances once per schedule run via [`Self::increment_change_tick`].
    change_tick: Tick,
    /// The tick at which trackers were last advanced. Together with
    /// `change_tick` it forms the default `(last, current)` window used by
    /// tick-aware accessors.
    last_change_tick: Tick,

    id: AtomicU64,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            entities: Entities::default(),
            archetypes: ArchetypeSet::new(),
            bundle_to_archetype: TypeIdMap::default(),
            insert_edges: IndexTypeIdMap::default(),
            remove_edges: IndexTypeIdMap::default(),
            resources: Resources::default(),
            command_queue: RefCell::new(Vec::new()),
            component_hooks: HashMap::new(),
            // The change clock starts at 1 so that a system's initial
            // `last_run` of 0 observes every component as new.
            change_tick: Tick::new(1),
            last_change_tick: Tick::new(0),
            id: AtomicU64::new(0),
        }
    }

    pub fn flush(&mut self) {
        // zero is reserved for entities with no components.
        let archetype = self.archetypes.get_mut(0);
        self.entities
            .flush(|id, location| location.index = unsafe { archetype.allocate(id) });
    }

    // ------------------------------------------------------------------
    // Change detection
    // ------------------------------------------------------------------

    /// Reads the world's current change tick.
    #[inline]
    pub fn change_tick(&self) -> Tick {
        self.change_tick
    }

    /// The tick at which change trackers were last advanced.
    #[inline]
    pub fn last_change_tick(&self) -> Tick {
        self.last_change_tick
    }

    /// Advances the change clock, returning the previous tick.
    ///
    /// The previous tick becomes [`Self::last_change_tick`], so the window
    /// `(last_change_tick, change_tick)` always spans exactly the writes made
    /// since the previous call.
    #[inline]
    pub fn increment_change_tick(&mut self) -> Tick {
        let prev = self.change_tick;
        self.last_change_tick = prev;
        self.change_tick = Tick::new(prev.get().wrapping_add(1));
        prev
    }

    /// Create an entity with certain components
    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
        self.flush();
        let entity = self.entities.alloc();
        self.spawn_inner(entity, components);
        entity
    }

    /// Place `entity` with `components`, allocating the id if it is free.
    ///
    /// If `entity` is currently alive, its previous components are dropped
    /// with the despawn hook sequence (`on_despawn` → `on_discard` →
    /// `on_remove`) so structural invariants (e.g. hierarchy links) stay
    /// intact; the new components then fire `on_add` → `on_insert` as usual.
    /// A hook that despawns the entity aborts the spawn gracefully.
    pub fn spawn_at(&mut self, entity: Entity, components: impl DynamicBundle) {
        self.flush();

        // Fire the despawn/discard hooks while the entity's location is still
        // valid (`alloc_at` invalidates it until `spawn_inner` re-places the
        // row, which would make the hooks' world access to the entity fail).
        let had_components = if let Ok(loc) = self.entities.get(entity) {
            let ids: Vec<TypeId> = self.archetypes.archetypes[loc.archetype as usize]
                .type_ids()
                .to_vec();
            for &id in &ids {
                self.fire_hook(HookKind::Despawn, id, entity);
            }
            for &id in &ids {
                self.fire_hook(HookKind::Discard, id, entity);
            }
            // A hook may have despawned the entity; in that case the pending
            // spawn aborts (the hook owns the outcome).
            if self.entities.get(entity).is_err() {
                return;
            }
            Some(ids)
        } else {
            None
        };

        // It is possible that entity already exists in this location.
        // If so, we need to remove it from the old location (dropping its
        // components — the hooks above already ran). Otherwise, we can just
        // insert it.
        if let Some(loc) = self.entities.alloc_at(entity) {
            if let Some(moved) = unsafe {
                self.archetypes
                    .get_mut(loc.archetype)
                    .remove(loc.index, true)
            } {
                self.entities.meta[moved as usize].location.index = loc.index;
            }
            // on_remove fires after the values are gone, mirroring despawn.
            if let Some(ids) = had_components {
                for id in ids {
                    self.fire_hook(HookKind::Remove, id, entity);
                }
            }
        }

        self.spawn_inner(entity, components);
    }

    fn spawn_inner(&mut self, entity: Entity, components: impl DynamicBundle) {
        let ids: Vec<TypeId> = components.with_ids(|ids| ids.to_vec());
        let archetype_id = match components.key() {
            Some(k) => *self
                .bundle_to_archetype
                .entry(k)
                .or_insert_with(|| self.archetypes.get_for(&components)),
            None => self.archetypes.get_for(&components),
        };

        let tick = self.change_tick;
        let index = unsafe {
            let archetype = self.archetypes.get_mut(archetype_id);
            let row = archetype.allocate(entity.id());
            components.put(|ptr, meta| {
                archetype.put_ptr(ptr, *meta.id(), meta.layout().size(), row);
                let column = archetype.get_state_by_id(meta.id()).unwrap();
                archetype.write_fresh_ticks(column, row, tick);
            });
            row
        };

        self.entities.meta[entity.id as usize].location = Location {
            archetype: archetype_id,
            index,
        };

        // Spawning counts as adding every component.
        for id in ids {
            self.fire_component_added(id, entity);
        }
    }

    pub fn spawn_batch<I>(&mut self, iter: I) -> SpawnBatchIter<'_, I::IntoIter>
    where
        I: IntoIterator,
        I::Item: Bundle + 'static,
    {
        // NOTE: the bulk path does not fire component lifecycle hooks (the
        // iterator borrows the world's storage directly, so there is no
        // consistent point to run `&mut World` callbacks from).
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
            tick: self.change_tick,
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
        // Resolve first so the despawn/discard hooks can fire while every
        // component is still in place.
        let loc = self.entities.get(entity)?;
        let ids: Vec<TypeId> = self.archetypes.archetypes[loc.archetype as usize]
            .type_ids()
            .to_vec();
        // Despawn hooks run first: linked-spawn relationship targets despawn
        // their sources here, and each source's own discard hook then unlinks
        // it from this entity's (still present) target collection.
        for &id in &ids {
            self.fire_hook(HookKind::Despawn, id, entity);
        }
        for &id in &ids {
            self.fire_hook(HookKind::Discard, id, entity);
        }
        // Re-resolve: a hook may have moved (or even despawned) the entity.
        let loc = self.entities.free(entity)?;
        if let Some(moved) =
            unsafe { self.archetypes.archetypes[loc.archetype as usize].remove(loc.index, true) }
        {
            self.entities.meta[moved as usize].location.index = loc.index;
        }
        for id in ids {
            self.fire_hook(HookKind::Remove, id, entity);
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

    /// Iterate over all currently live entities.
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.iter()
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

    /// Whether a resource of type `R` exists in the world.
    pub fn contains_resource<R: crate::Resource>(&self) -> bool {
        self.resources.contains::<R>()
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
    pub fn query<'a, Q: WorldQuery>(&'a self) -> Q::Iter<'a> {
        Q::fetch(self)
    }

    /// Query the world for a mutable combination of components.
    pub fn query_mut<'a, Q: WorldQuery>(&'a mut self) -> Q::Iter<'a> {
        Q::fetch_mut(self)
    }

    /// Query with an archetype filter
    /// ([`With`](crate::With)/[`Without`](crate::Without)/[`Or`](crate::Or)).
    pub fn query_filtered<'a, Q: WorldQuery, F: crate::QueryFilter>(&'a self) -> Q::Iter<'a> {
        Q::fetch_with(self, &crate::system::archetype_matches::<F>)
    }

    /// Mutable query with an archetype filter.
    pub fn query_filtered_mut<'a, Q: WorldQuery, F: crate::QueryFilter>(
        &'a mut self,
    ) -> Q::Iter<'a> {
        Q::fetch_mut_with(self, &crate::system::archetype_matches::<F>)
    }

    // ------------------------------------------------------------------
    // Commands (deferred structural mutation queue)
    // ------------------------------------------------------------------

    /// Queue a deferred world mutation. Used by [`Commands`](crate::Commands);
    /// applied by [`Self::apply_commands`].
    pub(crate) fn queue_command(&self, command: Command) {
        self.command_queue.borrow_mut().push(command);
    }

    /// Drain the deferred command queue, applying every queued mutation in
    /// order. Commands queued by other commands are applied within the same
    /// pass. The schedule runner calls this after every system, so a system's
    /// commands are visible to the systems that run after it.
    pub fn apply_commands(&mut self) {
        loop {
            let batch = mem::take(self.command_queue.get_mut());
            if batch.is_empty() {
                break;
            }
            for command in batch {
                command(self);
            }
        }
    }

    // ------------------------------------------------------------------
    // Internals (crate-visible for the archetype query engine)
    // ------------------------------------------------------------------

    pub(crate) fn raw_archetypes(&self) -> &[Archetype] {
        &self.archetypes.archetypes
    }

    pub(crate) fn raw_entity_meta(&self) -> &[crate::entities::EntityMeta] {
        &self.entities.meta
    }

    /// Locate an entity as `(archetype index, row)`, if it exists.
    pub(crate) fn locate_entity(&self, entity: Entity) -> Option<(usize, u32)> {
        let loc = self.entities.get(entity).ok()?;
        Some((loc.archetype as usize, loc.index))
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

    /// Get a tick-aware shared reference to `T` on `entity`, if present.
    ///
    /// The returned [`Ref`] reports added/changed relative to the world's
    /// default window `(last_change_tick, change_tick)`.
    pub fn get_component_ref<T: Component>(&self, entity: Entity) -> Option<Ref<'_, T>> {
        let loc = self.entities.get(entity).ok()?;
        let arch = &self.archetypes.archetypes[loc.archetype as usize];
        let col = arch.get_state::<T>()?;
        // SAFETY: same reasoning as `get_component`; the tick row is parallel
        // to the component row and equally in bounds.
        let value = unsafe { &*arch.get_base::<T>(col).as_ptr().add(loc.index as usize) };
        let ticks = unsafe { arch.read_ticks(col, loc.index) };
        Some(Ref::new(
            value,
            ticks,
            self.last_change_tick,
            self.change_tick,
        ))
    }

    /// Get a tick-aware unique reference to `T` on `entity`, if present.
    ///
    /// Mutably dereferencing the returned [`Mut`] records the world's current
    /// change tick on the component.
    pub fn get_component_mut<T: Component>(&mut self, entity: Entity) -> Option<Mut<'_, T>> {
        // Resolve the location first so the borrow of `self.entities` is
        // released before we take a mutable borrow of the archetype slice.
        let loc = self.entities.get(entity).ok()?;
        let last_run = self.last_change_tick;
        let this_run = self.change_tick;
        let archetypes = &mut self.archetypes.archetypes;
        let arch = &mut archetypes[loc.archetype as usize];
        let col = arch.get_state::<T>()?;
        let base = unsafe { arch.get_base::<T>(col) };
        let ticks = unsafe { arch.ticks_base(col) };
        // SAFETY: `base`/`ticks` are derived from `arch` (mutably borrowed via
        // `&mut self`); the row is within bounds and no other reference can
        // exist.
        Some(unsafe {
            Mut::new(
                base.as_ptr().add(loc.index as usize),
                ticks.as_ptr().add(loc.index as usize),
                last_run,
                this_run,
            )
        })
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
        // Replacing an existing component: fire the discard hook while the
        // old value is still in place.
        if self
            .entities
            .get(entity)
            .ok()
            .is_some_and(|loc| self.archetypes.archetypes[loc.archetype as usize].has::<T>())
        {
            self.fire_hook(HookKind::Discard, TypeId::of::<T>(), entity);
        }
        // Re-resolve: a hook may have moved (or despawned) the entity.
        let loc = self.entities.get(entity).ok()?;
        let old_id = loc.archetype;
        let old_row = loc.index;
        let tick = self.change_tick;

        // Replace in place when the archetype already holds `T`.
        let old_arch = &self.archetypes.archetypes[old_id as usize];
        if let Some(col) = old_arch.get_state::<T>() {
            // SAFETY: `&mut self` guarantees no aliasing `&T`/`&mut T` here.
            unsafe { *old_arch.get_base::<T>(col).as_ptr().add(old_row as usize) = value };
            let old_arch = &mut self.archetypes.archetypes[old_id as usize];
            // The value was replaced: bump changed, preserve added.
            let mut ticks = unsafe { old_arch.read_ticks(col, old_row) };
            ticks.changed = tick;
            old_arch.write_ticks(col, old_row, ticks);
            self.fire_hook(HookKind::Insert, TypeId::of::<T>(), entity);
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
        // new target row, carrying their change ticks along.
        // SAFETY: both archetypes are live; old_row/new_row are in bounds; we are
        // copying from a distinct allocation into the freshly allocated target row.
        unsafe {
            let old = &*old_raw;
            for meta in old.component_metas() {
                let size = meta.layout().size();
                if let Some(src) = old.get_ptr(*meta.id(), size, old_row) {
                    (*target_raw).put_ptr(src.as_ptr(), *meta.id(), size, new_row);
                    let old_col = old.get_state_by_id(meta.id()).unwrap();
                    let new_col = (*target_raw).get_state_by_id(meta.id()).unwrap();
                    let ticks = old.read_ticks(old_col, old_row);
                    (*target_raw).write_ticks(new_col, new_row, ticks);
                }
            }
            // Write the newly inserted component value with fresh ticks.
            let mut value = value;
            (*target_raw).put_ptr(
                (&mut value as *mut T).cast(),
                TypeId::of::<T>(),
                mem::size_of::<T>(),
                new_row,
            );
            mem::forget(value);
            let new_col = (*target_raw).get_state_by_id(&TypeId::of::<T>()).unwrap();
            (*target_raw).write_fresh_ticks(new_col, new_row, tick);
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

        // Newly added component: on_add then on_insert.
        self.fire_component_added(TypeId::of::<T>(), entity);
        Some(())
    }

    /// Insert (or replace) every component of `components` on `entity` in a
    /// single archetype move.
    ///
    /// Components the entity already has are replaced in value (changed tick
    /// bumped, added tick preserved); new components are added with fresh
    /// ticks. Returns `None` if `entity` does not exist.
    pub fn insert_bundle(&mut self, entity: Entity, components: impl DynamicBundle) -> Option<()> {
        self.flush();
        // Which bundle components already exist (will be replaced)? Fire their
        // discard hooks while the old values are still in place.
        let bundle_ids: Vec<TypeId> = components.with_ids(|ids| ids.to_vec());
        let mut replaced: Vec<TypeId> = Vec::new();
        match self.entities.get(entity) {
            Ok(loc) => {
                let arch = &self.archetypes.archetypes[loc.archetype as usize];
                for &id in &bundle_ids {
                    if arch.has_in_runtime(id) {
                        replaced.push(id);
                    }
                }
            }
            Err(_) => return None,
        }
        for &id in &replaced {
            self.fire_hook(HookKind::Discard, id, entity);
        }
        // Re-resolve: a hook may have moved (or despawned) the entity.
        let loc = self.entities.get(entity).ok()?;
        let old_id = loc.archetype;
        let old_row = loc.index;
        let tick = self.change_tick;

        // Target type set = old set ∪ bundle set, in archetype key order.
        let old_arch = &self.archetypes.archetypes[old_id as usize];
        let mut metas: Vec<ComponentMeta> = old_arch.component_metas().to_vec();
        for meta in components.component_meta() {
            if !metas.contains(&meta) {
                metas.push(meta);
            }
        }
        metas.sort_unstable();
        let ids: Box<[TypeId]> = metas.iter().map(|m| *m.id()).collect();
        let target_id = self.archetypes.get(ids, || metas.clone());

        // SAFETY: `target_id` may equal `old_id` (pure replacement); the raw
        // pointers below are used with disjoint rows in that case, and with
        // disjoint archetypes otherwise. They are taken after `get`, which may
        // have reallocated the archetype list.
        let old_raw: *const Archetype = &self.archetypes.archetypes[old_id as usize];
        let target_raw: *mut Archetype = &mut self.archetypes.archetypes[target_id as usize];

        // SAFETY: allocate a fresh row in the target archetype (no outstanding
        // borrows on its columns).
        let new_row = unsafe { (*target_raw).allocate(entity.id) };

        // Copy every old component the bundle does not replace, carrying ticks.
        // SAFETY: rows are in bounds; source and destination rows are disjoint.
        unsafe {
            let old = &*old_raw;
            for meta in old.component_metas() {
                if components.with_ids(|ids| ids.contains(meta.id())) {
                    continue;
                }
                let size = meta.layout().size();
                if let Some(src) = old.get_ptr(*meta.id(), size, old_row) {
                    (*target_raw).put_ptr(src.as_ptr(), *meta.id(), size, new_row);
                    let old_col = old.get_state_by_id(meta.id()).unwrap();
                    let new_col = (*target_raw).get_state_by_id(meta.id()).unwrap();
                    let ticks = old.read_ticks(old_col, old_row);
                    (*target_raw).write_ticks(new_col, new_row, ticks);
                }
            }
        }

        // Write the bundle's values. Replaced components keep their added tick
        // and bump changed; their old value is dropped in place (its slot is
        // logically consumed, so the swap-remove below must not drop it again).
        // SAFETY: as above; `put` hands each component value to the callback
        // exactly once, and `put_ptr` moves it into the archetype.
        unsafe {
            let old = &*old_raw;
            components.put(|ptr, meta| {
                let size = meta.layout().size();
                (*target_raw).put_ptr(ptr, *meta.id(), size, new_row);
                let new_col = (*target_raw).get_state_by_id(meta.id()).unwrap();
                if let Some(old_col) = old.get_state_by_id(meta.id()) {
                    let mut ticks = old.read_ticks(old_col, old_row);
                    ticks.changed = tick;
                    (*target_raw).write_ticks(new_col, new_row, ticks);
                    if let Some(old_slot) = old.get_ptr(*meta.id(), size, old_row) {
                        meta.drop_in_place(old_slot.as_ptr());
                    }
                } else {
                    (*target_raw).write_fresh_ticks(new_col, new_row, tick);
                }
            });
        }

        self.entities.meta[entity.id as usize].location = Location {
            archetype: target_id,
            index: new_row,
        };

        // Remove the old row without dropping: untouched components were moved
        // out above, replaced ones were already dropped. When target == old the
        // swap-remove packs the new row into the old slot, and the moved-entity
        // fixup below updates this entity's own index.
        // SAFETY: no outstanding borrows on the old archetype's columns.
        if let Some(moved) =
            unsafe { self.archetypes.archetypes[old_id as usize].remove(old_row, false) }
        {
            self.entities.meta[moved as usize].location.index = old_row;
        }

        for id in bundle_ids {
            if replaced.contains(&id) {
                self.fire_hook(HookKind::Insert, id, entity);
            } else {
                self.fire_component_added(id, entity);
            }
        }
        Some(())
    }

    /// Remove component `T` from `entity`, moving it to the archetype without
    /// `T` and returning the removed value.
    ///
    /// Returns `None` if `entity` does not exist or does not have `T`.
    pub fn remove_component<T: Component>(&mut self, entity: Entity) -> Option<T> {
        self.flush();
        // Fire the discard hook while the value is still in place.
        if self
            .entities
            .get(entity)
            .ok()
            .is_some_and(|loc| self.archetypes.archetypes[loc.archetype as usize].has::<T>())
        {
            self.fire_hook(HookKind::Discard, TypeId::of::<T>(), entity);
        }
        // Re-resolve: a hook may have moved (or despawned) the entity.
        let loc = self.entities.get(entity).ok()?;
        let old_id = loc.archetype;
        let old_row = loc.index;

        let old_arch = &self.archetypes.archetypes[old_id as usize];
        let col = old_arch.get_state::<T>()?;
        // Move the value out of its column (the swap-remove below must not
        // drop it again).
        // SAFETY: `&mut self` guarantees no aliasing access; the row is live.
        let value = unsafe {
            old_arch
                .get_base::<T>(col)
                .as_ptr()
                .add(old_row as usize)
                .read()
        };

        // Target type set = old set − {T} (order preserved by removal).
        let metas: Vec<ComponentMeta> = old_arch
            .component_metas()
            .iter()
            .copied()
            .filter(|m| m.id() != &TypeId::of::<T>())
            .collect();
        let ids: Box<[TypeId]> = metas.iter().map(|m| *m.id()).collect();
        let target_id = self.archetypes.get(ids, || metas.clone());
        debug_assert_ne!(target_id, old_id);

        // SAFETY: distinct archetypes; pointers taken after `get`.
        let old_raw: *const Archetype = &self.archetypes.archetypes[old_id as usize];
        let target_raw: *mut Archetype = &mut self.archetypes.archetypes[target_id as usize];
        // SAFETY: no outstanding borrows on the target archetype's columns.
        let new_row = unsafe { (*target_raw).allocate(entity.id) };

        // Move every remaining component's bytes and ticks to the target row.
        // SAFETY: rows in bounds, disjoint allocations.
        unsafe {
            let old = &*old_raw;
            for meta in old.component_metas() {
                if meta.id() == &TypeId::of::<T>() {
                    continue;
                }
                let size = meta.layout().size();
                if let Some(src) = old.get_ptr(*meta.id(), size, old_row) {
                    (*target_raw).put_ptr(src.as_ptr(), *meta.id(), size, new_row);
                    let old_col = old.get_state_by_id(meta.id()).unwrap();
                    let new_col = (*target_raw).get_state_by_id(meta.id()).unwrap();
                    let ticks = old.read_ticks(old_col, old_row);
                    (*target_raw).write_ticks(new_col, new_row, ticks);
                }
            }
        }

        self.entities.meta[entity.id as usize].location = Location {
            archetype: target_id,
            index: new_row,
        };

        // No drop: T was read out above, the rest were moved.
        // SAFETY: no outstanding borrows on the old archetype's columns.
        if let Some(moved) =
            unsafe { self.archetypes.archetypes[old_id as usize].remove(old_row, false) }
        {
            self.entities.meta[moved as usize].location.index = old_row;
        }

        self.fire_hook(HookKind::Remove, TypeId::of::<T>(), entity);
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iter_entities_reflects_spawn_and_despawn() {
        let mut world = World::new();
        let a = world.spawn_empty();
        let b = world.spawn_empty();
        let c = world.spawn_empty();

        world.despawn(b).unwrap();
        // Reuses b's freed id with a bumped generation.
        let d = world.spawn_empty();

        let live: Vec<Entity> = world.iter_entities().collect();
        assert_eq!(live.len(), 3);
        assert!(live.contains(&a));
        assert!(live.contains(&c));
        assert!(live.contains(&d));
        assert!(!live.contains(&b));
        assert!(world.iter_entities().all(|e| world.contains(e)));
    }

    #[test]
    fn test_iter_entities_empty_world() {
        let world = World::new();
        assert_eq!(world.iter_entities().count(), 0);
    }
}

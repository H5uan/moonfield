use crate::archetype::Archetype;
use crate::change_detection::{Mut, Tick};
use crate::entities::EntityMeta;
use crate::{Component, Entity, World};

/// Archetype-level predicate consulted when a query iterator is built:
/// archetypes it rejects contribute no entities. This is how
/// [`QueryFilter`](crate::QueryFilter) (`With`/`Without`/`Or`) is applied.
pub(crate) type ArchetypeFilter<'f> = &'f dyn Fn(&Archetype) -> bool;

/// A type-erased iterator over the world's archetypes, tied to a borrow of the
/// world so that yielded references outlive every `next` call.
///
/// Named after Bevy's `WorldQuery`: this is the low-level query description
/// (`&T`, `&mut T`, tuples), distinct from the [`Query`](crate::Query) system
/// param that wraps it.
pub trait WorldQuery {
    /// The item produced per matching entity.
    type Item<'w>: 'w
    where
        Self: 'w;
    /// The concrete iterator type, yielding `(Entity, Self::Item<'w>)`.
    type Iter<'w>: Iterator<Item = (Entity, Self::Item<'w>)> + 'w
    where
        Self: 'w;

    /// Build an iterator borrowing the world immutably.
    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch_with(world, &|_| true)
    }

    /// Build an iterator, skipping archetypes rejected by `filter`.
    #[doc(hidden)]
    fn fetch_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w;

    /// Build an iterator borrowing the world mutably.
    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        // SAFETY: the returned iterator and every item it yields borrow
        // `world` through `'w`, so no conflicting access to the fetched
        // columns can be created while they are alive.
        unsafe { Self::fetch_mut_cell(world) }
    }

    /// `fetch_mut` with an archetype filter.
    fn fetch_mut_with<'w>(world: &'w mut World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        // SAFETY: same argument as `fetch_mut`.
        unsafe { Self::fetch_mut_cell_with(world, filter) }
    }

    /// Build the mutable iterator from a *shared* world reference.
    ///
    /// Used by the [`Query`](crate::Query) system param, which is fetched from
    /// a shared borrow so it can coexist with the system's other params.
    ///
    /// # Safety
    ///
    /// The returned iterator takes the archetype borrow flags at construction
    /// but releases them when the *iterator* drops, while the items it yields
    /// (e.g. `Mut<'w, T>`) stay valid for `'w`. The caller must guarantee that
    /// no conflicting access to the fetched columns happens while the iterator
    /// **or any item produced by it** is still alive — in practice, that the
    /// items' lifetimes are tied to an exclusive borrow that also gates every
    /// other access to the same columns (as `&mut World` and
    /// `Query::iter_mut`'s `&mut self` do).
    #[doc(hidden)]
    unsafe fn fetch_mut_cell<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        // SAFETY: forwarded from the caller.
        unsafe { Self::fetch_mut_cell_with(world, &|_| true) }
    }

    /// `fetch_mut_cell` with an archetype filter.
    ///
    /// # Safety
    ///
    /// Same contract as [`fetch_mut_cell`](Self::fetch_mut_cell).
    #[doc(hidden)]
    unsafe fn fetch_mut_cell_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w;

    /// The item produced by per-entity access ([`Query::get`](crate::Query::get)):
    /// a guard that dereferences to the component and releases its column
    /// borrow flag on drop.
    type EntityFetch<'w>: 'w
    where
        Self: 'w;

    /// Fetch the item for a single entity, if it matches the query.
    ///
    /// Implemented for the single-component shapes (`&T`, `&mut T`); tuple
    /// and `Option` shapes panic — port them when a caller needs them.
    #[doc(hidden)]
    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<Self::EntityFetch<'w>>
    where
        Self: 'w;
}

// ---------------------------------------------------------------------
// Per-entity access guards (Query::get)
// ---------------------------------------------------------------------

/// Guard produced by per-entity shared access (`Query<&T>::get`).
///
/// Dereferences to `&T`; the column's shared borrow flag is released on drop.
pub struct EntityRef<'w, T: Component> {
    value: &'w T,
    archetype: &'w Archetype,
    column: usize,
}

impl<T: Component> std::ops::Deref for EntityRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<T: Component> Drop for EntityRef<'_, T> {
    fn drop(&mut self) {
        self.archetype.release::<T>(self.column);
    }
}

/// Guard produced by per-entity mutable access (`Query<&mut T>::get`).
///
/// Dereferences to `Mut<T>` (and thus `T`); the column's unique borrow flag is
/// released on drop.
pub struct EntityMut<'w, T: Component> {
    inner: Mut<'w, T>,
    archetype: &'w Archetype,
    column: usize,
}

impl<T: Component> std::ops::Deref for EntityMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: Component> std::ops::DerefMut for EntityMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T: Component> Drop for EntityMut<'_, T> {
    fn drop(&mut self) {
        self.archetype.release_mut::<T>(self.column);
    }
}

/// Shared per-entity fetch for single-component queries.
fn get_entity_ref<'w, T: Component>(world: &'w World, entity: Entity) -> Option<EntityRef<'w, T>> {
    let (arch_i, row) = world.locate_entity(entity)?;
    let archetype = &world.raw_archetypes()[arch_i];
    let column = archetype.get_state::<T>()?;
    archetype.borrow::<T>(column);
    // SAFETY: the column is shared-borrowed above and the row is live.
    let value = unsafe { &*archetype.get_base::<T>(column).as_ptr().add(row as usize) };
    Some(EntityRef {
        value,
        archetype,
        column,
    })
}

/// Mutable per-entity fetch for single-component queries.
fn get_entity_mut<'w, T: Component>(world: &'w World, entity: Entity) -> Option<EntityMut<'w, T>> {
    let (arch_i, row) = world.locate_entity(entity)?;
    let archetype = &world.raw_archetypes()[arch_i];
    let column = archetype.get_state::<T>()?;
    archetype.borrow_mut::<T>(column);
    let base = unsafe { archetype.get_base::<T>(column) };
    let ticks = unsafe { archetype.ticks_base(column) };
    // SAFETY: the column is uniquely borrowed above and the row is live; both
    // the component row and its tick row are exclusively ours until the guard
    // drops.
    let inner = unsafe {
        Mut::new(
            base.as_ptr().add(row as usize),
            ticks.as_ptr().add(row as usize),
            world.last_change_tick(),
            world.change_tick(),
        )
    };
    Some(EntityMut {
        inner,
        archetype,
        column,
    })
}

fn get_entity_unsupported<Q>() -> Option<Q> {
    panic!("per-entity `Query::get` is only implemented for `&T` and `&mut T` queries")
}

// ---------------------------------------------------------------------
// Single component, shared: `&T`
// ---------------------------------------------------------------------

pub struct ArchIter<'w, T: Component> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    // (archetype index, column index) of every archetype that holds T.
    // Columns are shared-borrowed for the lifetime of this iterator.
    hits: Vec<(usize, usize)>,
    ai: usize,
    row: u32,
    _marker: std::marker::PhantomData<&'w T>,
}

impl<'w, T: Component> ArchIter<'w, T> {
    fn new(world: &'w World, filter: ArchetypeFilter) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if !filter(a) {
                continue;
            }
            if let Some(col) = a.get_state::<T>() {
                a.borrow::<T>(col);
                hits.push((i, col));
            }
        }
        Self {
            meta,
            archetypes,
            hits,
            ai: 0,
            row: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: Component> Drop for ArchIter<'_, T> {
    fn drop(&mut self) {
        for &(i, col) in &self.hits {
            self.archetypes[i].release::<T>(col);
        }
    }
}

impl<'w, T: Component> Iterator for ArchIter<'w, T> {
    type Item = (Entity, &'w T);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &(arch_i, col) = self.hits.get(self.ai)?;
            let arch = &self.archetypes[arch_i];
            if self.row < arch.len() {
                let entity = self.entity_at(arch, self.row);
                let ptr = unsafe { arch.get_base::<T>(col) };
                let out: &'w T = unsafe { &*ptr.as_ptr().add(self.row as usize) };
                self.row += 1;
                return Some((entity, out));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<'w, T: Component> ArchIter<'w, T> {
    #[inline]
    fn entity_at(&self, arch: &Archetype, row: u32) -> Entity {
        let raw = arch.entity_id(row);
        Entity {
            id: raw,
            generation: self.meta[raw as usize].generation,
        }
    }
}

impl<T: Component> WorldQuery for &T {
    type Item<'w>
        = &'w T
    where
        Self: 'w;
    type Iter<'w>
        = ArchIter<'w, T>
    where
        Self: 'w;

    fn fetch_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        ArchIter::new(world, filter)
    }

    unsafe fn fetch_mut_cell_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch_with(world, filter)
    }

    type EntityFetch<'w>
        = EntityRef<'w, T>
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<Self::EntityFetch<'w>>
    where
        Self: 'w,
    {
        get_entity_ref::<T>(world, entity)
    }
}

// ---------------------------------------------------------------------
// Single mutable item: `&mut T`
// ---------------------------------------------------------------------

pub struct MutArchIter<'w, T: Component> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    hits: Vec<(usize, usize)>,
    ai: usize,
    row: u32,
    last_run: Tick,
    this_run: Tick,
    _marker: std::marker::PhantomData<&'w mut T>,
}

impl<'w, T: Component> MutArchIter<'w, T> {
    fn new(world: &'w World, filter: ArchetypeFilter) -> Self {
        let last_run = world.last_change_tick();
        let this_run = world.change_tick();
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if !filter(a) {
                continue;
            }
            if let Some(col) = a.get_state::<T>() {
                a.borrow_mut::<T>(col);
                hits.push((i, col));
            }
        }
        Self {
            meta,
            archetypes,
            hits,
            ai: 0,
            row: 0,
            last_run,
            this_run,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: Component> Drop for MutArchIter<'_, T> {
    fn drop(&mut self) {
        for &(i, col) in &self.hits {
            self.archetypes[i].release_mut::<T>(col);
        }
    }
}

impl<'w, T: Component> Iterator for MutArchIter<'w, T> {
    type Item = (Entity, Mut<'w, T>);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &(arch_i, col) = self.hits.get(self.ai)?;
            let arch = &self.archetypes[arch_i];
            if self.row < arch.len() {
                let raw = arch.entity_id(self.row);
                let entity = Entity {
                    id: raw,
                    generation: self.meta[raw as usize].generation,
                };
                let ptr = unsafe { arch.get_base::<T>(col) };
                let ticks = unsafe { arch.ticks_base(col) };
                // SAFETY: the column is uniquely borrowed for 'w, so both the
                // component row and its tick row are exclusively ours.
                let out = unsafe {
                    Mut::new(
                        ptr.as_ptr().add(self.row as usize),
                        ticks.as_ptr().add(self.row as usize),
                        self.last_run,
                        self.this_run,
                    )
                };
                self.row += 1;
                return Some((entity, out));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<T: Component> WorldQuery for &mut T {
    type Item<'w>
        = Mut<'w, T>
    where
        Self: 'w;
    type Iter<'w>
        = MutArchIter<'w, T>
    where
        Self: 'w;

    fn fetch_with<'w>(_world: &'w World, _filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        // A mutable component cannot be obtained from an immutable world.
        unreachable!("`&mut T` query requires a mutable world (`query_mut`)")
    }

    unsafe fn fetch_mut_cell_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        MutArchIter::new(world, filter)
    }

    type EntityFetch<'w>
        = EntityMut<'w, T>
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<Self::EntityFetch<'w>>
    where
        Self: 'w,
    {
        get_entity_mut::<T>(world, entity)
    }
}

// ---------------------------------------------------------------------
// Two immutable components: `(&A, &B)`
// ---------------------------------------------------------------------

pub struct PairIter<'w, A: Component, B: Component> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    hits: Vec<(usize, usize, usize)>, // (arch index, col A, col B)
    ai: usize,
    row: u32,
    _marker: std::marker::PhantomData<(&'w A, &'w B)>,
}

impl<'w, A: Component, B: Component> PairIter<'w, A, B> {
    fn new_shared(world: &'w World, filter: ArchetypeFilter) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if !filter(a) {
                continue;
            }
            if let (Some(ca), Some(cb)) = (a.get_state::<A>(), a.get_state::<B>()) {
                a.borrow::<A>(ca);
                a.borrow::<B>(cb);
                hits.push((i, ca, cb));
            }
        }
        Self {
            meta,
            archetypes,
            hits,
            ai: 0,
            row: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<A: Component, B: Component> Drop for PairIter<'_, A, B> {
    fn drop(&mut self) {
        for &(i, ca, cb) in &self.hits {
            self.archetypes[i].release::<A>(ca);
            self.archetypes[i].release::<B>(cb);
        }
    }
}

impl<'w, A: Component, B: Component> Iterator for PairIter<'w, A, B> {
    type Item = (Entity, (&'w A, &'w B));
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &(arch_i, ca, cb) = self.hits.get(self.ai)?;
            let arch = &self.archetypes[arch_i];
            if self.row < arch.len() {
                let raw = arch.entity_id(self.row);
                let entity = Entity {
                    id: raw,
                    generation: self.meta[raw as usize].generation,
                };
                let pa = unsafe { arch.get_base::<A>(ca) };
                let pb = unsafe { arch.get_base::<B>(cb) };
                let a: &'w A = unsafe { &*pa.as_ptr().add(self.row as usize) };
                let b: &'w B = unsafe { &*pb.as_ptr().add(self.row as usize) };
                self.row += 1;
                return Some((entity, (a, b)));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<A: Component, B: Component> WorldQuery for (&A, &B) {
    type Item<'w>
        = (&'w A, &'w B)
    where
        Self: 'w;
    type Iter<'w>
        = PairIter<'w, A, B>
    where
        Self: 'w;

    fn fetch_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        PairIter::new_shared(world, filter)
    }

    unsafe fn fetch_mut_cell_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch_with(world, filter)
    }

    type EntityFetch<'w>
        = ()
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<Self::EntityFetch<'w>>
    where
        Self: 'w,
    {
        let _ = (world, entity);
        get_entity_unsupported()
    }
}

// ---------------------------------------------------------------------
// Mutable + immutable pair: `(&mut A, &B)`
// ---------------------------------------------------------------------

pub struct MutSharedIter<'w, A: Component, B: Component> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    hits: Vec<(usize, usize, usize)>,
    ai: usize,
    row: u32,
    last_run: Tick,
    this_run: Tick,
    _marker: std::marker::PhantomData<(&'w mut A, &'w B)>,
}

impl<'w, A: Component, B: Component> MutSharedIter<'w, A, B> {
    fn new(world: &'w World, filter: ArchetypeFilter) -> Self {
        let last_run = world.last_change_tick();
        let this_run = world.change_tick();
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if !filter(a) {
                continue;
            }
            if let (Some(ca), Some(cb)) = (a.get_state::<A>(), a.get_state::<B>()) {
                a.borrow_mut::<A>(ca);
                a.borrow::<B>(cb);
                hits.push((i, ca, cb));
            }
        }
        Self {
            meta,
            archetypes,
            hits,
            ai: 0,
            row: 0,
            last_run,
            this_run,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<A: Component, B: Component> Drop for MutSharedIter<'_, A, B> {
    fn drop(&mut self) {
        for &(i, ca, cb) in &self.hits {
            self.archetypes[i].release_mut::<A>(ca);
            self.archetypes[i].release::<B>(cb);
        }
    }
}

impl<'w, A: Component, B: Component> Iterator for MutSharedIter<'w, A, B> {
    type Item = (Entity, (Mut<'w, A>, &'w B));
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &(arch_i, ca, cb) = self.hits.get(self.ai)?;
            let arch = &self.archetypes[arch_i];
            if self.row < arch.len() {
                let raw = arch.entity_id(self.row);
                let entity = Entity {
                    id: raw,
                    generation: self.meta[raw as usize].generation,
                };
                let pa = unsafe { arch.get_base::<A>(ca) };
                let ta = unsafe { arch.ticks_base(ca) };
                let pb = unsafe { arch.get_base::<B>(cb) };
                // SAFETY: column A is uniquely borrowed, column B shared.
                let a = unsafe {
                    Mut::new(
                        pa.as_ptr().add(self.row as usize),
                        ta.as_ptr().add(self.row as usize),
                        self.last_run,
                        self.this_run,
                    )
                };
                let b: &'w B = unsafe { &*pb.as_ptr().add(self.row as usize) };
                self.row += 1;
                return Some((entity, (a, b)));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<A: Component, B: Component> WorldQuery for (&mut A, &B) {
    type Item<'w>
        = (Mut<'w, A>, &'w B)
    where
        Self: 'w;
    type Iter<'w>
        = MutSharedIter<'w, A, B>
    where
        Self: 'w;

    fn fetch_with<'w>(_world: &'w World, _filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        unreachable!("`(&mut A, &B)` requires `query_mut`")
    }

    unsafe fn fetch_mut_cell_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        MutSharedIter::new(world, filter)
    }

    type EntityFetch<'w>
        = ()
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<Self::EntityFetch<'w>>
    where
        Self: 'w,
    {
        let _ = (world, entity);
        get_entity_unsupported()
    }
}

// ---------------------------------------------------------------------
// Two mutable components: `(&mut A, &mut B)`
// ---------------------------------------------------------------------

pub struct MutBothIter<'w, A: Component, B: Component> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    hits: Vec<(usize, usize, usize)>,
    ai: usize,
    row: u32,
    last_run: Tick,
    this_run: Tick,
    _marker: std::marker::PhantomData<(&'w mut A, &'w mut B)>,
}

impl<'w, A: Component, B: Component> MutBothIter<'w, A, B> {
    fn new(world: &'w World, filter: ArchetypeFilter) -> Self {
        let last_run = world.last_change_tick();
        let this_run = world.change_tick();
        let meta = world.raw_entity_meta();
        let archetypes = world.raw_archetypes();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if !filter(a) {
                continue;
            }
            if let (Some(ca), Some(cb)) = (a.get_state::<A>(), a.get_state::<B>()) {
                a.borrow_mut::<A>(ca);
                a.borrow_mut::<B>(cb);
                hits.push((i, ca, cb));
            }
        }
        Self {
            meta,
            archetypes,
            hits,
            ai: 0,
            row: 0,
            last_run,
            this_run,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<A: Component, B: Component> Drop for MutBothIter<'_, A, B> {
    fn drop(&mut self) {
        for &(i, ca, cb) in &self.hits {
            self.archetypes[i].release_mut::<A>(ca);
            self.archetypes[i].release_mut::<B>(cb);
        }
    }
}

impl<'w, A: Component, B: Component> Iterator for MutBothIter<'w, A, B> {
    type Item = (Entity, (Mut<'w, A>, Mut<'w, B>));
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &(arch_i, ca, cb) = self.hits.get(self.ai)?;
            let arch = &self.archetypes[arch_i];
            if self.row < arch.len() {
                let raw = arch.entity_id(self.row);
                let entity = Entity {
                    id: raw,
                    generation: self.meta[raw as usize].generation,
                };
                let pa = unsafe { arch.get_base::<A>(ca) };
                let ta = unsafe { arch.ticks_base(ca) };
                let pb = unsafe { arch.get_base::<B>(cb) };
                let tb = unsafe { arch.ticks_base(cb) };
                // SAFETY: both columns are uniquely borrowed for 'w.
                let a = unsafe {
                    Mut::new(
                        pa.as_ptr().add(self.row as usize),
                        ta.as_ptr().add(self.row as usize),
                        self.last_run,
                        self.this_run,
                    )
                };
                let b = unsafe {
                    Mut::new(
                        pb.as_ptr().add(self.row as usize),
                        tb.as_ptr().add(self.row as usize),
                        self.last_run,
                        self.this_run,
                    )
                };
                self.row += 1;
                return Some((entity, (a, b)));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<A: Component, B: Component> WorldQuery for (&mut A, &mut B) {
    type Item<'w>
        = (Mut<'w, A>, Mut<'w, B>)
    where
        Self: 'w;
    type Iter<'w>
        = MutBothIter<'w, A, B>
    where
        Self: 'w;

    fn fetch_with<'w>(_world: &'w World, _filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        unreachable!("`(&mut A, &mut B)` requires `query_mut`")
    }

    unsafe fn fetch_mut_cell_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        MutBothIter::new(world, filter)
    }

    type EntityFetch<'w>
        = ()
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<Self::EntityFetch<'w>>
    where
        Self: 'w,
    {
        let _ = (world, entity);
        get_entity_unsupported()
    }
}

// ---------------------------------------------------------------------
// Optional shared component: `Option<&T>`
// ---------------------------------------------------------------------

pub struct OptionIter<'w, T: Component> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    /// Archetype indices passing the filter (the iteration set).
    included: Vec<usize>,
    /// Archetype indices that contain T (these columns are shared-borrowed).
    borrowed: Vec<usize>,
    ai: usize,
    row: u32,
    _marker: std::marker::PhantomData<&'w T>,
}

impl<'w, T: Component> OptionIter<'w, T> {
    fn new(world: &'w World, filter: ArchetypeFilter) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut included = Vec::new();
        let mut borrowed = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if !filter(a) {
                continue;
            }
            included.push(i);
            if let Some(col) = a.get_state::<T>() {
                a.borrow::<T>(col);
                borrowed.push(i);
            }
        }
        Self {
            meta,
            archetypes,
            included,
            borrowed,
            ai: 0,
            row: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: Component> Drop for OptionIter<'_, T> {
    fn drop(&mut self) {
        for &i in &self.borrowed {
            let col = self.archetypes[i].get_state::<T>().unwrap();
            self.archetypes[i].release::<T>(col);
        }
    }
}

impl<'w, T: Component> Iterator for OptionIter<'w, T> {
    type Item = (Entity, Option<&'w T>);
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let &arch_i = self.included.get(self.ai)?;
            let arch = &self.archetypes[arch_i];
            if self.row < arch.len() {
                let raw = arch.entity_id(self.row);
                let entity = Entity {
                    id: raw,
                    generation: self.meta[raw as usize].generation,
                };
                let value = if let Some(col) = arch.get_state::<T>() {
                    let ptr = unsafe { arch.get_base::<T>(col) };
                    Some(unsafe { &*ptr.as_ptr().add(self.row as usize) })
                } else {
                    None
                };
                self.row += 1;
                return Some((entity, value));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<T: Component> WorldQuery for Option<&T> {
    type Item<'w>
        = Option<&'w T>
    where
        Self: 'w;
    type Iter<'w>
        = OptionIter<'w, T>
    where
        Self: 'w;

    fn fetch_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        OptionIter::new(world, filter)
    }

    unsafe fn fetch_mut_cell_with<'w>(world: &'w World, filter: ArchetypeFilter) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch_with(world, filter)
    }

    type EntityFetch<'w>
        = ()
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<Self::EntityFetch<'w>>
    where
        Self: 'w,
    {
        let _ = (world, entity);
        get_entity_unsupported()
    }
}

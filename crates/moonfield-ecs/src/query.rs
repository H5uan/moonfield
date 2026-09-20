//! The query engine: composable [`WorldQuery`] elements — `&T`, `&mut T`,
//! `Option<Q>`, and tuples of either — iterated by one generic
//! [`QueryIter`], ported from Bevy's `WorldQuery` composition at the
//! mechanism level.
//!
//! Each element decides which archetypes contribute entities
//! ([`WorldQuery::matches`]), borrows its columns for the iterator's
//! lifetime ([`WorldQuery::borrow_fetch`] / [`WorldQuery::release`], the
//! archetype borrow flags), and produces one item per row
//! ([`WorldQuery::fetch`]). Tuples compose all three conjunctively;
//! `Option<Q>` matches every archetype and yields `None` rows where `Q`'s
//! column is absent. The [`Query`](crate::Query) system param and
//! [`World::query`] are thin entries over the same iterator; filters
//! ([`With`](crate::With)/[`Without`](crate::Without)/[`Or`](crate::Or)) are
//! applied as an archetype predicate alongside `matches`.
//!
//! Items borrow the world for `'w` and may outlive the iterator that
//! produced them — and with it the release of the column borrow flags.
//! Conflicting component access between sibling [`Query`](crate::Query)
//! params therefore cannot wait for the flags: each `Query` registers its
//! access in the world's [`AccessRegistry`] when fetched and unregisters on
//! drop, so a read/write or write/write overlap between params panics at
//! fetch time.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};

use crate::archetype::Archetype;
use crate::change_detection::{Mut, Tick};
use crate::entities::EntityMeta;
use crate::{Component, Entity, World};

/// Archetype-level predicate consulted when a query iterator is built:
/// archetypes it rejects contribute no entities. This is how
/// [`QueryFilter`](crate::QueryFilter) (`With`/`Without`/`Or`) is applied.
pub(crate) type ArchetypeFilter<'f> = &'f dyn Fn(&Archetype) -> bool;

/// The component access of the live [`Query`](crate::Query) system params of
/// one world, keyed by component [`TypeId`].
///
/// A `Query` param registers its components when it is fetched
/// ([`SystemParam::fetch`](crate::SystemParam::fetch)) and unregisters them
/// when it drops, so the registry always reflects exactly the params alive
/// right now. Registering read access over a live write — or write access
/// over any live access — panics at fetch time, before any iteration: query
/// items may outlive the iterator that produced them (and with it the column
/// borrow flags), so a conflict between sibling params cannot wait for the
/// flags to catch it. The type is public only because it appears in the
/// [`WorldQuery`] protocol's signatures; it is not a usable API.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct AccessRegistry {
    /// Number of live read registrations per component.
    reads: HashMap<TypeId, usize>,
    /// Components with a live write registration (at most one each).
    writes: HashSet<TypeId>,
}

impl AccessRegistry {
    fn register_read(&mut self, id: TypeId, name: &'static str) {
        assert!(
            !self.writes.contains(&id),
            "conflicting `Query` params: `{name}` is read while another live param writes it"
        );
        *self.reads.entry(id).or_insert(0) += 1;
    }

    fn register_write(&mut self, id: TypeId, name: &'static str) {
        assert!(
            !self.reads.contains_key(&id),
            "conflicting `Query` params: `{name}` is written while another live param reads it"
        );
        assert!(
            self.writes.insert(id),
            "conflicting `Query` params: `{name}` is written by more than one live param"
        );
    }

    fn unregister_read(&mut self, id: TypeId) {
        if let Some(count) = self.reads.get_mut(&id) {
            *count -= 1;
            if *count == 0 {
                self.reads.remove(&id);
            }
        }
    }

    fn unregister_write(&mut self, id: TypeId) {
        self.writes.remove(&id);
    }
}

/// A composable query element: the low-level query description, distinct
/// from the [`Query`](crate::Query) system param that wraps it.
///
/// The engine calls the three-step protocol for every archetype the filter
/// accepts: [`Self::matches`] decides membership, [`Self::borrow_fetch`]
/// takes the element's column borrows (released by [`Self::release`] when
/// the iterator drops), and [`Self::fetch`] produces one item per row.
pub trait WorldQuery {
    /// The item produced per matching entity.
    type Item<'w>: 'w
    where
        Self: 'w;
    /// Per-archetype column state, built by [`Self::borrow_fetch`] and
    /// released by [`Self::release`].
    type Fetch<'w>: 'w
    where
        Self: 'w;

    /// Whether this query is read-only (`&T` everywhere, no `&mut T`).
    /// Read-only queries iterate from a shared world borrow; queries with
    /// mutable access need the exclusive entries (`World::query_mut`,
    /// `Query::iter_mut`).
    #[doc(hidden)]
    const READ_ONLY: bool;

    /// Register this element's component access (read for `&T`, write for
    /// `&mut T`) into `registry`, panicking on a conflict with already-live
    /// access. Called once per `Query` param fetch; registration of one
    /// element must not be observable if a later element of the same query
    /// panics (the caller stages registrations in a scratch copy).
    #[doc(hidden)]
    fn register_access(registry: &mut AccessRegistry);

    /// Undo one [`Self::register_access`] call. Called from `Query`'s `Drop`,
    /// so it must not panic.
    #[doc(hidden)]
    fn unregister_access(registry: &mut AccessRegistry);

    /// Whether archetype `a` contributes entities to this query.
    #[doc(hidden)]
    fn matches(a: &Archetype) -> bool;

    /// Borrow this element's columns in `a` and build the fetch. Only called
    /// on archetypes [`Self::matches`] accepted.
    #[doc(hidden)]
    fn borrow_fetch<'w>(a: &'w Archetype, last_run: Tick, this_run: Tick) -> Self::Fetch<'w>
    where
        Self: 'w;

    /// Release the borrows taken by [`Self::borrow_fetch`].
    #[doc(hidden)]
    fn release<'w>(fetch: &Self::Fetch<'w>, a: &'w Archetype)
    where
        Self: 'w;

    /// Produce the item for `row` in `a`.
    ///
    /// # Safety
    ///
    /// `row` must be within `a`'s length, and the fetch's column borrows
    /// must be held for `'w` by the caller (the iterator).
    #[doc(hidden)]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, a: &'w Archetype, row: u32) -> Self::Item<'w>
    where
        Self: 'w;

    /// Fetch the item for a single entity, if the entity's archetype
    /// matches: one entity's pass through the iteration protocol, with the
    /// borrow flags held by the returned [`QueryGetGuard`] until it drops.
    #[doc(hidden)]
    fn get_entity<'w>(
        world: &'w World,
        entity: Entity,
        last_run: Tick,
        this_run: Tick,
    ) -> Option<QueryGetGuard<'w, Self>>
    where
        Self: Sized + 'w,
    {
        let (arch_i, row) = world.locate_entity(entity)?;
        let archetype = &world.raw_archetypes()[arch_i];
        if !Self::matches(archetype) {
            return None;
        }
        let fetch = Self::borrow_fetch(archetype, last_run, this_run);
        // SAFETY: `row` is the entity's live row within the archetype, and
        // the fetch's column borrows are held by the returned guard until
        // it drops.
        let item = unsafe { Self::fetch(&fetch, archetype, row) };
        Some(QueryGetGuard {
            item,
            fetch,
            archetype,
        })
    }
}

// ---------------------------------------------------------------------
// Per-entity access guard (Query::get)
// ---------------------------------------------------------------------

/// Per-entity query item with its column borrows — the value
/// [`Query::get`](crate::Query::get) returns for any query shape, built by
/// running one entity through the iteration protocol.
///
/// The guard holds the fetch's borrow flags until it drops, so the item
/// (references into the entity's columns) stays sound while the guard
/// lives. Dereference to use the item; mutable elements go through
/// `DerefMut`, marking change ticks like iteration does.
pub struct QueryGetGuard<'w, Q: WorldQuery + 'w> {
    item: Q::Item<'w>,
    fetch: Q::Fetch<'w>,
    archetype: &'w Archetype,
}

impl<Q: WorldQuery> Drop for QueryGetGuard<'_, Q> {
    fn drop(&mut self) {
        Q::release(&self.fetch, self.archetype);
    }
}

impl<'w, Q: WorldQuery + 'w> std::ops::Deref for QueryGetGuard<'w, Q> {
    type Target = Q::Item<'w>;

    fn deref(&self) -> &Self::Target {
        &self.item
    }
}

impl<'w, Q: WorldQuery + 'w> std::ops::DerefMut for QueryGetGuard<'w, Q> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.item
    }
}

/// Per-entity shared access guard (`Query<&T>::get`) — the single-component
/// shape of [`QueryGetGuard`].
pub type EntityRef<'w, T> = QueryGetGuard<'w, &'w T>;

/// Per-entity mutable access guard (`Query<&mut T>::get`) — the
/// single-component shape of [`QueryGetGuard`]; `is_added`/`is_changed`
/// arrive through the [`Mut`](crate::Mut) item.
pub type EntityMut<'w, T> = QueryGetGuard<'w, &'w mut T>;

// ---------------------------------------------------------------------
// Elements: `&T`, `&mut T`, `Option<Q>`, tuples
// ---------------------------------------------------------------------

/// Column index of a shared `&T` element.
pub struct ReadFetch {
    column: usize,
}

impl<T: Component> WorldQuery for &T {
    type Item<'w>
        = &'w T
    where
        Self: 'w;
    type Fetch<'w>
        = ReadFetch
    where
        Self: 'w;

    const READ_ONLY: bool = true;

    fn register_access(registry: &mut AccessRegistry) {
        registry.register_read(TypeId::of::<T>(), std::any::type_name::<T>());
    }

    fn unregister_access(registry: &mut AccessRegistry) {
        registry.unregister_read(TypeId::of::<T>());
    }

    fn matches(a: &Archetype) -> bool {
        a.get_state::<T>().is_some()
    }

    fn borrow_fetch<'w>(a: &'w Archetype, _last_run: Tick, _this_run: Tick) -> ReadFetch
    where
        Self: 'w,
    {
        let column = a.get_state::<T>().expect("`matches` guarantees the column");
        a.borrow::<T>(column);
        ReadFetch { column }
    }

    fn release<'w>(fetch: &ReadFetch, a: &'w Archetype)
    where
        Self: 'w,
    {
        a.release::<T>(fetch.column);
    }

    unsafe fn fetch<'w>(fetch: &ReadFetch, a: &'w Archetype, row: u32) -> &'w T
    where
        Self: 'w,
    {
        // SAFETY: the column is shared-borrowed for 'w and the row is within
        // the archetype's length.
        unsafe { &*a.get_base::<T>(fetch.column).as_ptr().add(row as usize) }
    }
}

/// Column index and change ticks of a unique `&mut T` element.
pub struct MutFetch {
    column: usize,
    last_run: Tick,
    this_run: Tick,
}

impl<T: Component> WorldQuery for &mut T {
    type Item<'w>
        = Mut<'w, T>
    where
        Self: 'w;
    type Fetch<'w>
        = MutFetch
    where
        Self: 'w;

    const READ_ONLY: bool = false;

    fn register_access(registry: &mut AccessRegistry) {
        registry.register_write(TypeId::of::<T>(), std::any::type_name::<T>());
    }

    fn unregister_access(registry: &mut AccessRegistry) {
        registry.unregister_write(TypeId::of::<T>());
    }

    fn matches(a: &Archetype) -> bool {
        a.get_state::<T>().is_some()
    }

    fn borrow_fetch<'w>(a: &'w Archetype, last_run: Tick, this_run: Tick) -> MutFetch
    where
        Self: 'w,
    {
        let column = a.get_state::<T>().expect("`matches` guarantees the column");
        a.borrow_mut::<T>(column);
        MutFetch {
            column,
            last_run,
            this_run,
        }
    }

    fn release<'w>(fetch: &MutFetch, a: &'w Archetype)
    where
        Self: 'w,
    {
        a.release_mut::<T>(fetch.column);
    }

    unsafe fn fetch<'w>(fetch: &MutFetch, a: &'w Archetype, row: u32) -> Mut<'w, T>
    where
        Self: 'w,
    {
        // SAFETY: the column is uniquely borrowed for 'w and the row is
        // within the archetype's length.
        unsafe {
            let base = a.get_base::<T>(fetch.column);
            let ticks = a.ticks_base(fetch.column);
            Mut::new(
                base.as_ptr().add(row as usize),
                ticks.as_ptr().add(row as usize),
                fetch.last_run,
                fetch.this_run,
            )
        }
    }
}

/// `Option<Q>`: matches every archetype; rows where `Q`'s column is absent
/// yield `None`.
impl<Q: WorldQuery> WorldQuery for Option<Q> {
    type Item<'w>
        = Option<Q::Item<'w>>
    where
        Self: 'w;
    type Fetch<'w>
        = Option<Q::Fetch<'w>>
    where
        Self: 'w;

    const READ_ONLY: bool = Q::READ_ONLY;

    fn register_access(registry: &mut AccessRegistry) {
        Q::register_access(registry);
    }

    fn unregister_access(registry: &mut AccessRegistry) {
        Q::unregister_access(registry);
    }

    fn matches(_a: &Archetype) -> bool {
        true
    }

    fn borrow_fetch<'w>(a: &'w Archetype, last_run: Tick, this_run: Tick) -> Option<Q::Fetch<'w>>
    where
        Self: 'w,
    {
        if Q::matches(a) {
            Some(Q::borrow_fetch(a, last_run, this_run))
        } else {
            None
        }
    }

    fn release<'w>(fetch: &Option<Q::Fetch<'w>>, a: &'w Archetype)
    where
        Self: 'w,
    {
        if let Some(fetch) = fetch {
            Q::release(fetch, a);
        }
    }

    unsafe fn fetch<'w>(
        fetch: &Option<Q::Fetch<'w>>,
        a: &'w Archetype,
        row: u32,
    ) -> Option<Q::Item<'w>>
    where
        Self: 'w,
    {
        match fetch {
            // SAFETY: forwarded from the caller (the iterator), which holds
            // the column borrows for 'w.
            Some(fetch) => Some(unsafe { Q::fetch(fetch, a, row) }),
            None => None,
        }
    }
}

macro_rules! impl_world_query_tuple {
    ($($q:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(clippy::unused_unit)] // the empty-tuple expansion produces `()`
        impl<$($q: WorldQuery),*> WorldQuery for ($($q,)*) {
            type Item<'w> = ($($q::Item<'w>,)*) where Self: 'w;
            type Fetch<'w> = ($($q::Fetch<'w>,)*) where Self: 'w;

            const READ_ONLY: bool = true $(&& $q::READ_ONLY)*;

            #[allow(unused_variables)] // the empty-tuple expansion ignores it
            fn register_access(registry: &mut AccessRegistry) {
                $($q::register_access(registry);)*
            }

            #[allow(unused_variables)] // the empty-tuple expansion ignores it
            fn unregister_access(registry: &mut AccessRegistry) {
                $($q::unregister_access(registry);)*
            }

            #[allow(unused_variables)] // the empty-tuple expansion ignores them
            fn matches(a: &Archetype) -> bool {
                true $(&& $q::matches(a))*
            }

            #[allow(unused_variables)] // the empty-tuple expansion ignores them
            fn borrow_fetch<'w>(
                a: &'w Archetype,
                last_run: Tick,
                this_run: Tick,
            ) -> Self::Fetch<'w>
            where
                Self: 'w,
            {
                ($($q::borrow_fetch(a, last_run, this_run),)*)
            }

            #[allow(unused_variables)] // the empty-tuple expansion ignores them
            fn release<'w>(fetch: &Self::Fetch<'w>, a: &'w Archetype)
            where
                Self: 'w,
            {
                let ($($q,)*) = fetch;
                $($q::release($q, a);)*
            }

            #[allow(unused_variables)] // the empty-tuple expansion ignores them
            #[allow(unused_unsafe)] // the empty-tuple expansion makes no calls
            unsafe fn fetch<'w>(
                fetch: &Self::Fetch<'w>,
                a: &'w Archetype,
                row: u32,
            ) -> Self::Item<'w>
            where
                Self: 'w,
            {
                let ($($q,)*) = fetch;
                // SAFETY: forwarded from the caller (the iterator), which
                // holds every element's column borrows for 'w.
                unsafe { ($($q::fetch($q, a, row),)*) }
            }
        }
    };
}

smaller_tuples_too!(impl_world_query_tuple, Q0, Q1, Q2, Q3, Q4, Q5, Q6, Q7);

// ---------------------------------------------------------------------
// The generic iterator
// ---------------------------------------------------------------------

/// The one query iterator: walks every archetype the filter and `Q::matches`
/// accept, holding each element's column borrows from construction to drop.
pub struct QueryIter<'w, Q: WorldQuery + 'w> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    /// (archetype index, fetch) of every matching archetype; the column
    /// borrows taken by the fetches live until this iterator drops.
    hits: Vec<(usize, Q::Fetch<'w>)>,
    ai: usize,
    row: u32,
}

impl<'w, Q: WorldQuery + 'w> QueryIter<'w, Q> {
    /// Build a read-only iterator, rejecting queries that contain mutable
    /// access — those need the exclusive entries (`World::query_mut`,
    /// `Query::iter_mut`).
    pub(crate) fn new_shared(
        world: &'w World,
        filter: ArchetypeFilter<'_>,
        last_run: Tick,
        this_run: Tick,
    ) -> Self {
        assert_shared::<Q>();
        // SAFETY: `Q` is read-only, so the fetches take only shared flags.
        unsafe { Self::new(world, filter, last_run, this_run) }
    }

    /// Build an iterator that may take unique column flags from a *shared*
    /// world reference.
    ///
    /// The `last_run`/`this_run` window is the caller's: a system's own
    /// window for the [`Query`](crate::Query) param, the world's default
    /// window for `World::query`.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that no conflicting access to the fetched
    /// columns happens while the iterator **or any item produced by it** is
    /// still alive — in practice, that an exclusive borrow gates every other
    /// access to the same columns (as `&mut World` and `Query::iter_mut`'s
    /// `&mut self` do).
    pub(crate) unsafe fn new(
        world: &'w World,
        filter: ArchetypeFilter<'_>,
        last_run: Tick,
        this_run: Tick,
    ) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if !filter(a) || !Q::matches(a) {
                continue;
            }
            hits.push((i, Q::borrow_fetch(a, last_run, this_run)));
        }
        Self {
            meta,
            archetypes,
            hits,
            ai: 0,
            row: 0,
        }
    }

    /// Build a read-only iterator over a precomputed match list (the
    /// per-system [`QueryState`](crate::system::QueryState) cache), skipping
    /// the archetype scan.
    pub(crate) fn new_shared_cached(
        world: &'w World,
        matched: &[u32],
        last_run: Tick,
        this_run: Tick,
    ) -> Self {
        assert_shared::<Q>();
        // SAFETY: `Q` is read-only, so the fetches take only shared flags.
        unsafe { Self::new_cached(world, matched, last_run, this_run) }
    }

    /// Build an iterator over a precomputed match list, possibly taking
    /// unique column flags from a *shared* world reference.
    ///
    /// `matched` must be exactly the archetype indices the query and its
    /// filter accept, in archetype-set order — the invariant
    /// [`QueryState`](crate::system::QueryState) maintains.
    ///
    /// # Safety
    ///
    /// Same contract as [`Self::new`].
    pub(crate) unsafe fn new_cached(
        world: &'w World,
        matched: &[u32],
        last_run: Tick,
        this_run: Tick,
    ) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::with_capacity(matched.len());
        for &i in matched {
            let a = &archetypes[i as usize];
            hits.push((i as usize, Q::borrow_fetch(a, last_run, this_run)));
        }
        Self {
            meta,
            archetypes,
            hits,
            ai: 0,
            row: 0,
        }
    }
}

impl<Q: WorldQuery> Drop for QueryIter<'_, Q> {
    fn drop(&mut self) {
        for (i, fetch) in &self.hits {
            Q::release(fetch, &self.archetypes[*i]);
        }
    }
}

impl<'w, Q: WorldQuery + 'w> Iterator for QueryIter<'w, Q> {
    type Item = (Entity, Q::Item<'w>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (arch_i, fetch) = self.hits.get(self.ai)?;
            let arch = &self.archetypes[*arch_i];
            if self.row < arch.len() {
                let raw = arch.entity_id(self.row);
                let entity = Entity {
                    id: raw,
                    generation: self.meta[raw as usize].generation,
                };
                // SAFETY: `row` is within `arch`'s length and the column
                // borrows are held by this iterator for 'w.
                let item = unsafe { Q::fetch(fetch, arch, self.row) };
                self.row += 1;
                return Some((entity, item));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

fn assert_shared<Q: WorldQuery>() {
    if !Q::READ_ONLY {
        panic!("query contains mutable access; use `query_mut` / `iter_mut`");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct A(u32);

    #[derive(Debug, Clone, PartialEq)]
    struct B(u32);

    #[derive(Debug, Clone, PartialEq)]
    struct C(u32);

    #[test]
    fn tuples_match_conjunctively() {
        let mut world = World::new();
        world.spawn((A(1), B(2), C(3)));
        world.spawn((A(4), B(5)));
        world.spawn((A(6),));

        assert_eq!(world.query::<(&A, &B, &C)>().count(), 1);

        let both: Vec<_> = world
            .query::<(&A, &B)>()
            .map(|(_, (a, b))| (a.0, b.0))
            .collect();
        assert_eq!(both, [(1, 2), (4, 5)]);
    }

    #[test]
    fn option_in_tuple_yields_none_for_missing() {
        let mut world = World::new();
        let both = world.spawn((A(1), B(2)));
        let only_a = world.spawn((A(3),));

        let mut seen = Vec::new();
        for (entity, (a, b)) in world.query::<(&A, Option<&B>)>() {
            seen.push((entity, a.0, b.map(|b| b.0)));
        }
        assert_eq!(seen.len(), 2);
        assert!(seen.contains(&(both, 1, Some(2))));
        assert!(seen.contains(&(only_a, 3, None)));
    }

    #[test]
    fn option_mut_in_tuple_via_query_mut() {
        let mut world = World::new();
        world.spawn((A(1), B(10)));
        world.spawn((A(2),));

        for (_, (mut a, b)) in world.query_mut::<(&mut A, Option<&B>)>() {
            if let Some(b) = b {
                a.0 += b.0;
            }
        }
        let values: Vec<_> = world.query::<&A>().map(|(_, a)| a.0).collect();
        assert_eq!(values, [11, 2]);
    }

    #[test]
    fn option_standalone_iterates_all_entities() {
        let mut world = World::new();
        world.spawn((A(1),));
        world.spawn((B(2),));
        assert_eq!(world.query::<Option<&A>>().count(), 2);
        assert_eq!(
            world
                .query::<Option<&A>>()
                .filter(|(_, a)| a.is_some())
                .count(),
            1
        );
    }

    #[test]
    #[should_panic(expected = "mutable access")]
    fn shared_query_rejects_mutable_access() {
        let mut world = World::new();
        world.spawn((A(1),));
        let _ = world.query::<(&mut A, Option<&B>)>();
    }

    #[test]
    fn tuple_and_option_get_per_entity() {
        use crate::{Query, SystemState};

        let mut world = World::new();
        let both = world.spawn((A(1), B(2)));
        let only_a = world.spawn((A(3),));
        let bare = world.spawn((B(9),));

        let mut state = SystemState::<Query<(&mut A, Option<&B>)>>::new();
        let query = state.get(&world);

        {
            let mut guard = query.get(both).expect("both components present");
            let (a, b) = &mut *guard;
            assert_eq!(a.0, 1);
            assert_eq!(b.unwrap().0, 2);
            a.0 += 10;
        }
        // The guard dropped and released every column borrow: a second get
        // on the same entity works and observes the write.
        {
            let guard = query.get(both).expect("both components present");
            let (a, b) = &*guard;
            assert_eq!(a.0, 11);
            assert_eq!(b.unwrap().0, 2);
        }

        // Option yields None for the missing column, matching iteration.
        let guard = query.get(only_a).expect("A present");
        let (a, b) = &*guard;
        assert_eq!(a.0, 3);
        assert!(b.is_none());

        // An entity whose archetype does not match the query shape: None.
        assert!(query.get(bare).is_none());
    }
}

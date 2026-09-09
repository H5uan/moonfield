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

use crate::archetype::Archetype;
use crate::change_detection::{Mut, Tick};
use crate::entities::EntityMeta;
use crate::{Component, Entity, World};

/// Archetype-level predicate consulted when a query iterator is built:
/// archetypes it rejects contribute no entities. This is how
/// [`QueryFilter`](crate::QueryFilter) (`With`/`Without`/`Or`) is applied.
pub(crate) type ArchetypeFilter<'f> = &'f dyn Fn(&Archetype) -> bool;

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

    type EntityFetch<'w>
        = EntityRef<'w, T>
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<EntityRef<'w, T>>
    where
        Self: 'w,
    {
        get_entity_ref::<T>(world, entity)
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

    type EntityFetch<'w>
        = EntityMut<'w, T>
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<EntityMut<'w, T>>
    where
        Self: 'w,
    {
        get_entity_mut::<T>(world, entity)
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

    type EntityFetch<'w>
        = ()
    where
        Self: 'w;

    fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<()>
    where
        Self: 'w,
    {
        let _ = (world, entity);
        get_entity_unsupported()
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

            type EntityFetch<'w> = () where Self: 'w;

            fn get_entity<'w>(world: &'w World, entity: Entity) -> Option<()>
            where
                Self: 'w,
            {
                let _ = (world, entity);
                get_entity_unsupported()
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
    pub(crate) fn new_shared(world: &'w World, filter: ArchetypeFilter<'_>) -> Self {
        assert_shared::<Q>();
        // SAFETY: `Q` is read-only, so the fetches take only shared flags.
        unsafe { Self::new(world, filter) }
    }

    /// Build an iterator that may take unique column flags from a *shared*
    /// world reference.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that no conflicting access to the fetched
    /// columns happens while the iterator **or any item produced by it** is
    /// still alive — in practice, that an exclusive borrow gates every other
    /// access to the same columns (as `&mut World` and `Query::iter_mut`'s
    /// `&mut self` do).
    pub(crate) unsafe fn new(world: &'w World, filter: ArchetypeFilter<'_>) -> Self {
        let last_run = world.last_change_tick();
        let this_run = world.change_tick();
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
}

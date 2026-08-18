use crate::archetype::Archetype;
use crate::entities::EntityMeta;
use crate::{Component, Entity, World};

/// A type-erased iterator over the world's archetypes, tied to a borrow of the
/// world so that yielded references outlive every `next` call.
pub trait Query {
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
        Self: 'w;

    /// Build an iterator borrowing the world mutably.
    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w;
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
    fn new(world: &'w World) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
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

impl<T: Component> Query for &T {
    type Item<'w>
        = &'w T
    where
        Self: 'w;
    type Iter<'w>
        = ArchIter<'w, T>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        ArchIter::new(world)
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
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
    _marker: std::marker::PhantomData<&'w mut T>,
}

impl<'w, T: Component> MutArchIter<'w, T> {
    fn new(world: &'w mut World) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
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
    type Item = (Entity, &'w mut T);
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
                let out: &'w mut T = unsafe { &mut *ptr.as_ptr().add(self.row as usize) };
                self.row += 1;
                return Some((entity, out));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<T: Component> Query for &mut T {
    type Item<'w>
        = &'w mut T
    where
        Self: 'w;
    type Iter<'w>
        = MutArchIter<'w, T>
    where
        Self: 'w;

    fn fetch<'w>(_world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        // A mutable component cannot be obtained from an immutable world.
        unreachable!("`&mut T` query requires a mutable world (`query_mut`)")
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        MutArchIter::new(world)
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
    fn new_shared(world: &'w World) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
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

impl<A: Component, B: Component> Query for (&A, &B) {
    type Item<'w>
        = (&'w A, &'w B)
    where
        Self: 'w;
    type Iter<'w>
        = PairIter<'w, A, B>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        PairIter::new_shared(world)
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
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
    _marker: std::marker::PhantomData<(&'w mut A, &'w B)>,
}

impl<'w, A: Component, B: Component> MutSharedIter<'w, A, B> {
    fn new(world: &'w mut World) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
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
    type Item = (Entity, (&'w mut A, &'w B));
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
                let a: &'w mut A = unsafe { &mut *pa.as_ptr().add(self.row as usize) };
                let b: &'w B = unsafe { &*pb.as_ptr().add(self.row as usize) };
                self.row += 1;
                return Some((entity, (a, b)));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<A: Component, B: Component> Query for (&mut A, &B) {
    type Item<'w>
        = (&'w mut A, &'w B)
    where
        Self: 'w;
    type Iter<'w>
        = MutSharedIter<'w, A, B>
    where
        Self: 'w;

    fn fetch<'w>(_world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        unreachable!("`(&mut A, &B)` requires `query_mut`")
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        MutSharedIter::new(world)
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
    _marker: std::marker::PhantomData<(&'w mut A, &'w mut B)>,
}

impl<'w, A: Component, B: Component> MutBothIter<'w, A, B> {
    fn new(world: &'w mut World) -> Self {
        let meta = world.raw_entity_meta();
        let archetypes = world.raw_archetypes();
        let mut hits = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
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
    type Item = (Entity, (&'w mut A, &'w mut B));
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
                let a: &'w mut A = unsafe { &mut *pa.as_ptr().add(self.row as usize) };
                let b: &'w mut B = unsafe { &mut *pb.as_ptr().add(self.row as usize) };
                self.row += 1;
                return Some((entity, (a, b)));
            }
            self.ai += 1;
            self.row = 0;
        }
    }
}

impl<A: Component, B: Component> Query for (&mut A, &mut B) {
    type Item<'w>
        = (&'w mut A, &'w mut B)
    where
        Self: 'w;
    type Iter<'w>
        = MutBothIter<'w, A, B>
    where
        Self: 'w;

    fn fetch<'w>(_world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        unreachable!("`(&mut A, &mut B)` requires `query_mut`")
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        MutBothIter::new(world)
    }
}

// ---------------------------------------------------------------------
// Optional shared component: `Option<&T>`
// ---------------------------------------------------------------------

pub struct OptionIter<'w, T: Component> {
    meta: &'w [EntityMeta],
    archetypes: &'w [Archetype],
    /// Archetype indices that contain T (these columns are shared-borrowed).
    borrowed: Vec<usize>,
    arch_i: usize,
    row: u32,
    _marker: std::marker::PhantomData<&'w T>,
}

impl<'w, T: Component> OptionIter<'w, T> {
    fn new(world: &'w World) -> Self {
        let archetypes = world.raw_archetypes();
        let meta = world.raw_entity_meta();
        let mut borrowed = Vec::new();
        for (i, a) in archetypes.iter().enumerate() {
            if let Some(col) = a.get_state::<T>() {
                a.borrow::<T>(col);
                borrowed.push(i);
            }
        }
        Self {
            meta,
            archetypes,
            borrowed,
            arch_i: 0,
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
            let arch = self.archetypes.get(self.arch_i)?;
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
            self.arch_i += 1;
            self.row = 0;
        }
    }
}

impl<T: Component> Query for Option<&T> {
    type Item<'w>
        = Option<&'w T>
    where
        Self: 'w;
    type Iter<'w>
        = OptionIter<'w, T>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        OptionIter::new(world)
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
    }
}

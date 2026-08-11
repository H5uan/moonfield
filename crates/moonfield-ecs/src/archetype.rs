//! Type-erased, archetype-based component storage.
//!
//! An [`Archetype`] is a collection of entities that share exactly the same set
//! of component types. Each component type's data lives in its own contiguous
//! column (SoA layout), so a query only touches the columns it needs and
//! iteration is cache-friendly.

use std::{
    alloc::{alloc, dealloc, handle_alloc_error, Layout},
    any::TypeId,
    collections::HashMap,
    hash::{BuildHasher, BuildHasherDefault, Hasher},
    ptr::{self, NonNull},
};

use crate::{borrow::AtomicBorrow, component_ref::ComponentRef, Component};

/// A [`Hasher`] that forwards the `TypeId` value directly.
///
/// `TypeId` is a `u128` under the hood, so we bypass the usual hashing pass and
/// use the id itself as the hash. This makes [`TypeIdMap`] lookups essentially
/// a direct index into the hash table with no collision work.
#[derive(Default)]
pub struct TypeIdHasher(u64);

impl Hasher for TypeIdHasher {
    fn write_u64(&mut self, n: u64) {
        // Only a single value can be hashed, so the old hash should be zero.
        debug_assert_eq!(self.0, 0);
        self.0 = n;
    }

    // Tolerate TypeId being either u64 or u128.
    fn write_u128(&mut self, n: u128) {
        debug_assert_eq!(self.0, 0);
        self.0 = n as u64;
    }

    fn write(&mut self, bytes: &[u8]) {
        debug_assert_eq!(self.0, 0);

        // This will only be called if TypeId is neither u64 nor u128, which is not anticipated.
        // In that case we'll just fall back to using a different hash implementation.
        let mut hasher = foldhash::fast::FixedState::with_seed(0xb334867b740a29a5).build_hasher();
        hasher.write(bytes);
        self.0 = hasher.finish();
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// A [`HashMap`] keyed by [`TypeId`] using [`TypeIdHasher`].
pub type TypeIdMap<V> = HashMap<TypeId, V, BuildHasherDefault<TypeIdHasher>>;

/// A fixed-size map from [`TypeId`] to a value, stored as a sorted array.
///
/// Unlike [`TypeIdMap`], this is allocated once with the full set of types an
/// archetype will ever hold, so it never grows and lookups are a binary search
/// over a contiguous slice. It is the primary "which column holds type `T`?"
/// index inside an [`Archetype`].
struct OrderedTypeIdMap<V>(Box<[(TypeId, V)]>);

impl<V> OrderedTypeIdMap<V> {
    /// Build the map from an iterator, sorting by `TypeId` for binary search.
    fn new(iter: impl Iterator<Item = (TypeId, V)>) -> Self {
        let mut vals = iter.collect::<Box<[_]>>();
        vals.sort_unstable_by_key(|(id, _)| *id);
        Self(vals)
    }

    /// Binary-search for the index of `id`.
    fn search(&self, id: &TypeId) -> Option<usize> {
        self.0.binary_search_by_key(id, |(id, _)| *id).ok()
    }

    /// Whether `id` is present.
    fn contains_key(&self, id: &TypeId) -> bool {
        self.search(id).is_some()
    }

    /// Get the value associated with `id`.
    fn get(&self, id: &TypeId) -> Option<&V> {
        self.search(id).map(move |idx| &self.0[idx].1)
    }
}

/// The per-type column of a single [`Archetype`].
///
/// `raw_data` is a contiguous untyped buffer: row `i` (at byte offset
/// `i * T::layout().size()`) holds the `T` component of entity `i` in this
/// archetype. `borrow_state` arbitrates shared (read) vs unique (write) access
/// to the whole column, which is what lets compile-time-safe queries be checked
/// at runtime.
struct Data {
    borrow_state: AtomicBorrow,
    raw_data: NonNull<u8>,
}

/// A type-erased, runtime description of a component type.
///
/// Everything needed to store, move, and destroy component values of a given
/// type without knowing it at compile time: the [`TypeId`] for identity, the
/// [`Layout`] for allocation, and a drop shim for teardown.
#[derive(Copy, Clone, Debug)]
pub struct ComponentMeta {
    id: TypeId,
    layout: Layout,
    drop_fn: unsafe fn(*mut u8),
    #[cfg(debug_assertions)]
    type_name: &'static str,
}

impl ComponentMeta {
    /// Construct a component meta for a given component type.
    pub fn of<T: 'static>() -> Self {
        unsafe fn drop_ptr<T>(x: *mut u8) {
            x.cast::<T>().drop_in_place();
        }

        Self {
            id: TypeId::of::<T>(),
            layout: Layout::new::<T>(),
            drop_fn: drop_ptr::<T>,
            #[cfg(debug_assertions)]
            type_name: core::any::type_name::<T>(),
        }
    }

    /// Construct a component meta from raw parts.
    ///
    /// Useful when the type information is obtained dynamically, e.g. from a
    /// bundle built at runtime.
    pub fn from_parts(id: TypeId, layout: Layout, drop: unsafe fn(*mut u8)) -> Self {
        Self {
            id,
            layout,
            drop_fn: drop,
            #[cfg(debug_assertions)]
            type_name: "<unknown> (TypeInfo constructed from parts)",
        }
    }

    /// Access the `TypeId` of the component type.
    pub fn id(&self) -> &TypeId {
        &self.id
    }

    /// Access the layout of the component type.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Directly call the destructor of the component type.
    ///
    /// # Safety
    ///
    /// `data` must point to a valid, initialized value of the component type
    /// described by this meta, and must not be used again afterward.
    pub unsafe fn drop_in_place(&self, data: *mut u8) {
        (self.drop_fn)(data)
    }

    /// The raw drop shim function pointer.
    pub fn drop_shim(&self) -> unsafe fn(*mut u8) {
        self.drop_fn
    }
}

impl PartialOrd for ComponentMeta {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComponentMeta {
    /// Order by alignment, descending. Ties broken with TypeId.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.layout
            .align()
            .cmp(&other.layout.align())
            .reverse()
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PartialEq for ComponentMeta {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for ComponentMeta {}

/// A collection of entities having the same component types.
///
/// Accessing `Archetype`s directly is only required in niche cases. Typical use
/// should go through the [`World`](crate::World).
pub struct Archetype {
    metas: Vec<ComponentMeta>,
    type_ids: Vec<TypeId>,
    index: OrderedTypeIdMap<usize>,
    /// Number of entities that allocated in this archetype.
    len: u32,
    /// Raw entity IDs, one per row. `entities.len()` doubles as the capacity.
    entities: Box<[u32]>,
    /// Raw data with atomic borrow state for each component type.
    data: Box<[Data]>,
}

impl Archetype {
    /// Validate that `metas` is sorted in [`ComponentMeta::cmp`] order and
    /// contains no duplicates.
    fn assert_component_meta(metas: &[ComponentMeta]) {
        metas.windows(2).for_each(|x| match x[0].cmp(&x[1]) {
            core::cmp::Ordering::Less => (),
            #[cfg(debug_assertions)]
            core::cmp::Ordering::Equal => panic!(
                "attempted to allocate entity with duplicate {} components; \
                 each type must occur at most once!",
                x[0].type_name
            ),
            #[cfg(not(debug_assertions))]
            core::cmp::Ordering::Equal => panic!(
                "attempted to allocate entity with duplicate components; \
                 each type must occur at most once!"
            ),
            core::cmp::Ordering::Greater => panic!("type info is unsorted"),
        });
    }

    /// Create a new archetype holding the given component types.
    ///
    /// `metas` must already be sorted by [`ComponentMeta::cmp`] (alignment
    /// descending, with `TypeId` as a tie-break) and contain no duplicates;
    /// this is verified at runtime.
    ///
    /// # Panics
    ///
    /// Panics if `metas` contains duplicate component types or is not sorted.
    pub(crate) fn new(metas: Vec<ComponentMeta>) -> Self {
        let max_align = metas.first().map_or(1, |meta| meta.layout.align());
        // Reject duplicate component types and enforce the alignment-descending
        // order that `ComponentMeta::cmp` guarantees, so `metas.first()` — and
        // thus `max_align` above — is the maximum alignment.
        Self::assert_component_meta(&metas);
        let component_count = metas.len();
        Self {
            index: OrderedTypeIdMap::new(metas.iter().enumerate().map(|(i, meta)| (meta.id, i))),
            type_ids: metas.iter().map(|meta| *meta.id()).collect(),
            metas,
            // Begin with zero capacity; the first `allocate` call grows.
            entities: Box::new([]),
            len: 0,
            data: (0..component_count)
                .map(|_| Data {
                    borrow_state: AtomicBorrow::new(),
                    // A placeholder non-null pointer; real storage is allocated
                    // on first grow, before any value is written.
                    raw_data: NonNull::new(max_align as *mut u8).unwrap(),
                })
                .collect(),
        }
    }

    /// Drop all components in this archetype and reset its length to zero.
    ///
    /// The underlying allocation is retained for reuse.
    pub(crate) fn clear(&mut self) {
        for (meta, column) in self.metas.iter().zip(&*self.data) {
            for index in 0..self.len {
                unsafe {
                    let removed = column
                        .raw_data
                        .as_ptr()
                        .add(index as usize * meta.layout.size());
                    (meta.drop_fn)(removed)
                }
            }
        }
        self.len = 0;
    }

    /// Whether this archetype contains a component with the given [`TypeId`].
    pub fn has_in_runtime(&self, id: TypeId) -> bool {
        self.index.contains_key(&id)
    }

    /// Whether this archetype contains `T` components.
    pub fn has<T: Component>(&self) -> bool {
        self.has_in_runtime(TypeId::of::<T>())
    }

    /// Find the column index associated with `T`, if present.
    ///
    /// The returned index is stable for the lifetime of the archetype and can
    /// be used with [`Self::get_base`].
    pub(crate) fn get_state<T: Component>(&self) -> Option<usize> {
        self.index.get(&TypeId::of::<T>()).copied()
    }

    /// Get the address of the first `T` component, given a state index from
    /// [`Self::get_state`].
    ///
    /// # Safety
    ///
    /// `column` must be associated with a component of type `T`.
    pub(crate) unsafe fn get_base<T: Component>(&self, column: usize) -> NonNull<T> {
        debug_assert_eq!(self.metas[column].id, TypeId::of::<T>());

        unsafe {
            NonNull::new_unchecked(
                self.data
                    .get_unchecked(column)
                    .raw_data
                    .as_ptr()
                    .cast::<T>(),
            )
        }
    }

    /// Borrow all components of a single type from these entities, if present.
    ///
    /// `T` must be a shared or unique reference to a component type. Useful for
    /// efficient serialization.
    pub fn get<'a, T: ComponentRef<'a>>(&'a self) -> Option<T::Column> {
        T::get_column(self)
    }

    /// Acquire a shared borrow on the column for `T`.
    ///
    /// # Panics
    ///
    /// Panics if the column is already uniquely (mutably) borrowed.
    pub(crate) fn borrow<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());
        if !self.data[column].borrow_state.try_borrow() {
            panic!(
                "Component {} is already borrowed",
                self.metas[column].type_name
            );
        }
    }

    /// Acquire a shared borrow on the column at `column`.
    ///
    /// Unlike [`Self::borrow`], this does not check the column's type.
    ///
    /// # Panics
    ///
    /// Panics if the column is already uniquely borrowed.
    pub(crate) unsafe fn borrow_raw(&self, column: usize) {
        if !self.data[column].borrow_state.try_borrow() {
            panic!(
                "Component {} is already borrowed",
                self.metas[column].type_name
            );
        }
    }

    /// Acquire a unique borrow on the column for `T`.
    ///
    /// # Panics
    ///
    /// Panics if the column is already borrowed (shared or unique).
    pub(crate) fn borrow_mut<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());

        if !self.data[column].borrow_state.try_borrow_mut() {
            panic!(
                "Component {} is already borrowed",
                self.metas[column].type_name
            );
        }
    }

    /// Release a shared borrow on the column for `T`.
    pub(crate) fn release<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());
        self.data[column].borrow_state.release_shared();
    }

    /// Release a unique borrow on the column for `T`.
    pub(crate) fn release_mut<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());
        self.data[column].borrow_state.release_unique();
    }

    /// Release a shared borrow on the column at `column`, without a type check.
    pub(crate) unsafe fn release_raw(&self, column: usize) {
        self.data[column].borrow_state.release_shared();
    }

    /// Release a unique borrow on the column at `column`, without a type check.
    pub(crate) unsafe fn release_raw_mut(&self, column: usize) {
        self.data[column].borrow_state.release_unique();
    }

    /// Number of entities in this archetype.
    #[inline]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Whether this archetype contains no entities.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get a pointer to the raw entity id array.
    ///
    /// Only the first `self.len()` entries are valid.
    #[inline]
    pub(crate) fn entities(&self) -> NonNull<u32> {
        unsafe { NonNull::new_unchecked(self.entities.as_ptr() as *mut _) }
    }

    /// Get the raw entity id at `index`.
    pub(crate) fn entity_id(&self, index: u32) -> u32 {
        self.entities[index as usize]
    }

    /// Overwrite the raw entity id at `index`.
    #[inline]
    pub(crate) fn set_entity_id(&mut self, index: usize, id: u32) {
        self.entities[index] = id;
    }

    /// The component metas of this archetype, in column order.
    pub(crate) fn component_metas(&self) -> &[ComponentMeta] {
        &self.metas
    }

    /// The `TypeId`s of this archetype's components, in column order.
    pub(crate) fn type_ids(&self) -> &[TypeId] {
        &self.type_ids
    }

    /// The number of rows this archetype can hold before reallocating.
    pub(crate) fn capacity(&self) -> u32 {
        self.entities.len() as u32
    }

    /// Enumerate the types of the components of entities stored in this
    /// archetype.
    ///
    /// Convenient for dispatching logic that must run over a set of type ids.
    /// For example, a scripting system could iterate the world's archetypes,
    /// extract every combination of component types currently stored, and map
    /// each archetype to a wrapper object that provides functionality based on
    /// its components.
    pub(crate) fn component_types(&self) -> impl ExactSizeIterator<Item = TypeId> + '_ {
        self.metas
            .iter()
            .map(|component_type_meta| component_type_meta.id)
    }

    /// Get a raw pointer to the component at row `index`, for the type
    /// identified by `ty` with row stride `size`.
    ///
    /// Returns `None` if this archetype does not contain `ty`.
    ///
    /// # Safety
    ///
    /// `index` must be `<= self.len()`, and the caller must ensure the column's
    /// borrow rules are respected.
    pub(crate) unsafe fn get_ptr(
        &self,
        ty: TypeId,
        size: usize,
        index: u32,
    ) -> Option<NonNull<u8>> {
        debug_assert!(index <= self.len());
        Some(NonNull::new_unchecked(
            self.data
                .get_unchecked(*self.index.get(&ty)?)
                .raw_data
                .as_ptr()
                .add(size * index as usize)
                .cast::<u8>(),
        ))
    }

    /// Increase capacity by exactly `increment`, reallocating every column.
    ///
    /// Newly extended space is left uninitialized.
    fn grow_exact(&mut self, increment: u32) {
        let old_count = self.len as usize;
        let old_cap = self.entities.len();
        let new_cap = old_cap + increment as usize;
        let mut new_entities = vec![!0; new_cap].into_boxed_slice();
        new_entities[0..old_count].copy_from_slice(&self.entities[0..old_count]);
        self.entities = new_entities;

        // Allocate a fresh, larger buffer for every column and copy the live
        // rows over. Only `self.data` is swapped in after every column has been
        // successfully allocated, so an OOM never leaves the archetype
        // partially reallocated.
        let new_data = self
            .metas
            .iter()
            .zip(&*self.data)
            .map(|(component_meta, old_data)| {
                let raw_data = if component_meta.layout.size() == 0 {
                    // Zero-sized types have no bytes to store; a dangling
                    // non-null pointer serves as their column.
                    NonNull::dangling()
                } else {
                    let layout = Layout::from_size_align(
                        component_meta.layout.size() * new_cap,
                        component_meta.layout.align(),
                    )
                    .unwrap();
                    unsafe {
                        let mem = alloc(layout);
                        let mem = NonNull::new(mem).unwrap_or_else(|| handle_alloc_error(layout));
                        ptr::copy_nonoverlapping(
                            old_data.raw_data.as_ptr(),
                            mem.as_ptr(),
                            component_meta.layout.size() * old_count,
                        );
                        mem
                    }
                };
                Data {
                    // `&mut self` guarantees no outstanding borrows, so the
                    // fresh borrow state starts unlocked.
                    borrow_state: AtomicBorrow::new(),
                    raw_data,
                }
            })
            .collect::<Box<[_]>>();

        // Now that the replacement is fully built, free the old column buffers.
        if old_cap > 0 {
            for (component_meta, data) in self.metas.iter().zip(&*self.data) {
                if component_meta.layout.size() == 0 {
                    continue;
                }
                unsafe {
                    std::alloc::dealloc(
                        data.raw_data.as_ptr(),
                        Layout::from_size_align(
                            component_meta.layout().size() * old_cap,
                            component_meta.layout.align(),
                        )
                        .unwrap(),
                    );
                }
            }
        }
        self.data = new_data;
    }

    /// Increase capacity by at least `min_increment`.
    ///
    /// Uses a Vec-like doubling strategy, making allocation amortized O(1).
    fn grow(&mut self, min_increment: u32) {
        // the same strategy as how Vec grows. Make O(1) amortized
        self.grow_exact(self.capacity().max(min_increment));
    }

    /// Append a new (empty) row for entity `id`, returning its row index.
    ///
    /// The caller must write every component type into the new row immediately
    /// after this call.
    ///
    /// # Safety
    ///
    /// No outstanding borrows may exist on any column.
    pub(crate) unsafe fn allocate(&mut self, id: u32) -> u32 {
        if self.len == self.capacity() {
            // magic number based on Experience
            self.grow(64);
        }

        self.entities[self.len as usize] = id;
        self.len += 1;
        self.len - 1
    }

    /// Set the number of live rows directly.
    ///
    /// # Safety
    ///
    /// `len` must be `<= self.capacity()`, and every row in `0..len` must be
    /// fully initialized.
    pub(crate) unsafe fn set_len(&mut self, len: u32) {
        debug_assert!(len <= self.capacity());
        self.len = len;
    }

    /// Ensure capacity for at least `additional` more rows.
    pub(crate) fn reserve(&mut self, additional: u32) {
        if additional > (self.capacity() - self.len()) {
            let increment = additional - (self.capacity() - self.len());
            self.grow(increment.max(64));
        }
    }

    /// Remove the entity at `index`, swapping in the last row to keep the
    /// storage packed.
    ///
    /// Returns the ID of the entity that was moved into `index`, if any.
    ///
    /// # Safety
    ///
    /// `index` must be in-bounds and no column may be borrowed.
    pub(crate) unsafe fn remove(&mut self, index: u32, drop: bool) -> Option<u32> {
        let last = self.len - 1;

        for (component_meta, data) in self.metas.iter().zip(&*self.data) {
            let removed = data
                .raw_data
                .as_ptr()
                .add(index as usize * component_meta.layout().size());
            if drop {
                (component_meta.drop_fn)(removed);
            }
            // use the last to overwrite the removed entity
            if index != last {
                let moved = data
                    .raw_data
                    .as_ptr()
                    .add(last as usize * component_meta.layout().size());
                ptr::copy_nonoverlapping(moved, removed, component_meta.layout().size());
            }
        }

        self.len = last;
        if index != last {
            self.entities[index as usize] = self.entities[last as usize];
            Some(self.entities[last as usize])
        } else {
            None
        }
    }

    /// Move every component of the entity at `index` out via `f`, then pack
    /// the last row into its place.
    ///
    /// Returns the ID of the entity moved into `index`, if any.
    ///
    /// # Safety
    ///
    /// `index` must be in-bounds and no column may be borrowed. `f` must not
    /// read or write the moved-out slots.
    pub(crate) unsafe fn move_to(
        &mut self,
        index: u32,
        mut f: impl FnMut(*mut u8, TypeId, usize),
    ) -> Option<u32> {
        let last = self.len - 1;
        for (component_meta, data) in self.metas.iter().zip(&*self.data) {
            let moved_out = data
                .raw_data
                .as_ptr()
                .add(index as usize * component_meta.layout.size());
            f(moved_out, component_meta.id, component_meta.layout().size());
            if index != last {
                let moved = data
                    .raw_data
                    .as_ptr()
                    .add(last as usize * component_meta.layout.size());
                ptr::copy_nonoverlapping(moved, moved_out, component_meta.layout.size());
            }
        }
        self.len -= 1;
        if index != last {
            self.entities[index as usize] = self.entities[last as usize];
            Some(self.entities[last as usize])
        } else {
            None
        }
    }

    /// Copy a single component value into the row at `index`.
    ///
    /// # Safety
    ///
    /// `component` must point to a valid value of the type identified by `ty`,
    /// and the row must have been allocated.
    pub(crate) unsafe fn put_ptr(
        &mut self,
        component: *mut u8,
        ty: TypeId,
        size: usize,
        index: u32,
    ) {
        let ptr = self.get_ptr(ty, size, index).unwrap().as_ptr().cast::<u8>();
        ptr::copy_nonoverlapping(component, ptr, size);
    }

    /// Add components from another archetype with identical components.
    ///
    /// Appends the rows of `other` to `self` in order, leaving `other` empty.
    ///
    /// # Safety
    ///
    /// Component types must match exactly.
    pub(crate) unsafe fn merge(&mut self, mut other: Archetype) {
        self.reserve(other.len);
        for ((info, dst), src) in self.metas.iter().zip(&*self.data).zip(&*other.data) {
            dst.raw_data
                .as_ptr()
                .add(self.len as usize * info.layout.size())
                .copy_from_nonoverlapping(
                    src.raw_data.as_ptr(),
                    other.len as usize * info.layout.size(),
                )
        }
        self.len += other.len;
        // Transfer ownership of the rows to `self`; `other` must not drop them.
        other.len = 0;
    }

    /// Raw IDs of the entities in this archetype.
    ///
    /// Convertible into [`Entity`](crate::Entity)s via the world's id lookup.
    /// Useful for efficient serialization.
    #[inline]
    pub fn ids(&self) -> &[u32] {
        &self.entities[0..self.len as usize]
    }
}

impl Drop for Archetype {
    fn drop(&mut self) {
        // Destroy every component value first ...
        self.clear();
        if self.entities.is_empty() {
            return;
        }

        // ... then free each column buffer.
        for (component_meta, data) in self.metas.iter().zip(&*self.data) {
            if component_meta.layout.size() != 0 {
                unsafe {
                    dealloc(
                        data.raw_data.as_ptr(),
                        Layout::from_size_align_unchecked(
                            component_meta.layout().size() * self.entities.len(),
                            component_meta.layout.align(),
                        ),
                    );
                }
            }
        }
    }
}

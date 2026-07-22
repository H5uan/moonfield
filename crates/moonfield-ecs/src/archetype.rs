use std::{
    alloc::Layout,
    any::TypeId,
    collections::HashMap,
    hash::{BuildHasher, BuildHasherDefault, Hasher},
    ptr::NonNull,
};

use crate::{borrow::AtomicBorrow, component_ref::ComponentRef, Component};

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

pub type TypeIdMap<V> = HashMap<TypeId, V, BuildHasherDefault<TypeIdHasher>>;

struct OrderedTypeIdMap<V>(Box<[(TypeId, V)]>);

impl<V> OrderedTypeIdMap<V> {
    fn new(iter: impl Iterator<Item = (TypeId, V)>) -> Self {
        let mut vals = iter.collect::<Box<[_]>>();
        vals.sort_unstable_by_key(|(id, _)| *id);
        Self(vals)
    }

    fn search(&self, id: &TypeId) -> Option<usize> {
        self.0.binary_search_by_key(id, |(id, _)| *id).ok()
    }

    fn contains_key(&self, id: &TypeId) -> bool {
        self.search(id).is_some()
    }

    fn get(&self, id: &TypeId) -> Option<&V> {
        self.search(id).map(move |idx| &self.0[idx].1)
    }
}

struct Column {
    borrow_state: AtomicBorrow,
    raw_data: NonNull<u8>,
}

/// A type-erased runtime desc for a component.
///
/// It transfer the compile-time type info to runtime.
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
    pub unsafe fn drop_in_place(&self, data: *mut u8) {
        (self.drop_fn)(data)
    }

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

pub struct Archetype {
    metas: Vec<ComponentMeta>,
    type_ids: Vec<TypeId>,
    column_of: OrderedTypeIdMap<usize>,
    len: u32,
    entities: Box<[u32]>,
    /// Raw data with atomic borrow state for each component type.
    data: Box<[Column]>,
}

impl Archetype {
    pub(crate) fn new(metas: Vec<ComponentMeta>) -> Self {
        let max_align = metas.first().map_or(1, |meta| meta.layout.align());
        let component_count = metas.len();
        Self {
            column_of: OrderedTypeIdMap::new(
                metas.iter().enumerate().map(|(i, meta)| (meta.id, i)),
            ),
            type_ids: metas.iter().map(|meta| *meta.id()).collect(),
            metas,
            entities: Box::new([]),
            len: 0,
            data: (0..component_count)
                .map(|_| Column {
                    borrow_state: AtomicBorrow::new(),
                    raw_data: NonNull::new(max_align as *mut u8).unwrap(),
                })
                .collect(),
        }
    }

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

    pub fn has_in_runtime(&self, id: TypeId) -> bool {
        self.column_of.contains_key(&id)
    }

    pub fn has<T: Component>(&self) -> bool {
        self.has_in_runtime(TypeId::of::<T>())
    }

    /// Get the type `T` corresponding column index.
    pub(crate) fn get_column<T: Component>(&self) -> Option<usize> {
        self.column_of.get(&TypeId::of::<T>()).copied()
    }

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

    pub fn get<'a, T: ComponentRef<'a>>(&'a self) -> Option<T::Column> {
        T::get_column(self)
    }

    pub(crate) fn borrow<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());
        if !self.data[column].borrow_state.try_borrow() {
            panic!(
                "Component {} is already borrowed",
                self.metas[column].type_name
            );
        }
    }

    pub(crate) unsafe fn borrow_raw(&self, column: usize) {
        if !self.data[column].borrow_state.try_borrow() {
            panic!(
                "Component {} is already borrowed",
                self.metas[column].type_name
            );
        }
    }

    pub(crate) fn borrow_mut<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());

        if !self.data[column].borrow_state.try_borrow_mut() {
            panic!(
                "Component {} is already borrowed",
                self.metas[column].type_name
            );
        }
    }

    pub(crate) fn release<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());
        self.data[column].borrow_state.release_shared();
    }

    pub(crate) fn release_mut<T: Component>(&self, column: usize) {
        assert_eq!(self.metas[column].id, TypeId::of::<T>());
        self.data[column].borrow_state.release_unique();
    }

    pub(crate) unsafe fn release_raw(&self, column: usize) {
        self.data[column].borrow_state.release_shared();
    }

    pub(crate) unsafe fn release_raw_mut(&self, column: usize) {
        self.data[column].borrow_state.release_unique();
    }

    /// Number of entities in this archetype
    #[inline]
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Whether this archetype contains no entities
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub(crate) fn entities(&self) -> NonNull<u32> {
        unsafe { NonNull::new_unchecked(self.entities.as_ptr() as *mut _) }
    }

    pub(crate) fn entity_id(&self, index: u32) -> u32 {
        self.entities[index as usize]
    }

    #[inline]
    pub(crate) fn set_entity_id(&mut self, index: usize, id: u32) {
        self.entities[index] = id;
    }

    pub(crate) fn component_metas(&self) -> &[ComponentMeta] {
        &self.metas
    }

    pub(crate) fn type_ids(&self) -> &[TypeId] {
        &self.type_ids
    }
}

use std::{alloc::Layout, any::TypeId, ptr::NonNull};

use crate::ecs::borrow::SharedRuntimeBorrow;

pub struct Archetype {
    types: Vec<TypeInfo>,
    type_ids: Box<[TypeId]>,
    index: OrderedTypeIdMap<usize>,
    len: u32,
    entities: Box<[u32]>,
    data: Box<[ArchetypeData]>,
}

struct ArchetypeData {
    state: SharedRuntimeBorrow,
    storage: NonNull<u8>,
}

/// Metadata for a component type
#[derive(Debug, Copy, Clone)]
pub struct TypeInfo {
    id: TypeId,
    layout: Layout,
    drop: unsafe fn(*mut u8),
    #[cfg(debug_assertions)]
    type_name: &'static str,
}

impl TypeInfo {
    /// Construct a `TypeInfo` directly from the static type.
    pub fn of<T: 'static>() -> Self {
        unsafe fn drop_ptr<T>(x: *mut u8) {
            x.cast::<T>().drop_in_place()
        }

        Self {
            id: TypeId::of::<T>(),
            layout: Layout::new::<T>(),
            drop: drop_ptr::<T>,
            #[cfg(debug_assertions)]
            type_name: core::any::type_name::<T>(),
        }
    }

    pub fn id(&self) -> TypeId {
        self.id
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub unsafe fn drop(&self, data: *mut u8) {
        (self.drop)(data)
    }
}

impl PartialEq for TypeInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for TypeInfo {}

impl PartialOrd for TypeInfo {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TypeInfo {
    /// Order by alignment, descending. Ties broken with TypeId.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.layout
            .align()
            .cmp(&other.layout.align())
            .reverse()
            .then_with(|| self.id.cmp(&other.id))
    }
}

/// A map from TypeId to values that preserves insertion order
pub struct OrderedTypeIdMap<V>(Vec<(TypeId, V)>);

impl<V> OrderedTypeIdMap<V> {
    pub fn new() -> Self {
        OrderedTypeIdMap(Vec::new())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        OrderedTypeIdMap(Vec::with_capacity(capacity))
    }

    pub fn insert(&mut self, id: TypeId, value: V) {
        // Check if already exists
        if let Some(pos) =
            self.0.iter().position(|(existing_id, _)| *existing_id == id)
        {
            self.0[pos] = (id, value);
        } else {
            self.0.push((id, value));
            // Sort by TypeId for binary search
            self.0.sort_unstable_by_key(|(id, _)| *id);
        }
    }

    pub fn get(&self, id: &TypeId) -> Option<&V> {
        self.0
            .binary_search_by_key(id, |(id, _)| *id)
            .ok()
            .map(|idx| &self.0[idx].1)
    }

    pub fn get_mut(&mut self, id: &TypeId) -> Option<&mut V> {
        if let Ok(idx) = self.0.binary_search_by_key(id, |(id, _)| *id) {
            Some(&mut self.0[idx].1)
        } else {
            None
        }
    }

    pub fn contains_key(&self, id: &TypeId) -> bool {
        self.get(id).is_some()
    }
}

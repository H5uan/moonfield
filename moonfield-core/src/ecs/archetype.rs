//-！ Archetype
//-!
//-! Archetype is a storage block for a collection of entities with same components
//-! In archetype, entities are stored as structure of arrays
use std::{alloc::Layout, any::TypeId, ptr::NonNull};

use crate::ecs::{borrow::SharedRuntimeBorrow, world::Component};

pub struct Archetype {
    /// meta data for all components
    types: Vec<TypeInfo>,
    type_ids: Box<[TypeId]>,
    index: OrderedTypeIdMap<usize>,
    len: u32,
    entities: Box<[u32]>,
    data: Box<[ArchetypeData]>,
}

impl Archetype {
    pub(crate) fn new(types: Vec<TypeInfo>) -> Self {
        let max_align =
            types.first().map_or(1, |type_info| type_info.layout.align());

        let component_count = types.len();
        Self {
            index: OrderedTypeIdMap::new(
                types
                    .iter()
                    .enumerate()
                    .map(|(i, type_info)| (type_info.id, i)),
            ),
            type_ids: types.iter().map(|type_info| type_info.id).collect(),
            types,
            entities: Box::new([]),
            len: 0,
            data: (0..component_count)
                .map(|_| ArchetypeData {
                    state: SharedRuntimeBorrow::new(),
                    storage: NonNull::new(max_align as *mut u8).unwrap(),
                })
                .collect(),
        }
    }

    pub(crate) fn clear(&mut self) {
        for (type_info, data) in self.types.iter().zip(&*self.data) {
            for index in 0..self.len{
                unsafe {
                    let removed = data.storage.as_ptr().add(index as usize * type_info.layout.size());
                    type_info.drop(removed);
                }
            }
        }
        self.len = 0;
    }

    pub fn has<T: Component>(&self) -> bool {
        self.has_dynamic(TypeId::of::<T>())
    }

    pub fn has_dynamic(&self, id: TypeId) -> bool {
        self.index.contains_key(&id)
    }

    

    
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
pub struct OrderedTypeIdMap<V>(Box<[(TypeId, V)]>);

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

use std::any::TypeId;
use std::borrow::Borrow;
use std::collections::{hash_map::Entry, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::AtomicU64;

use crate::archetype::{Archetype, ComponentMeta, TypeIdMap};
use crate::bundle::DynamicBundle;
use crate::entities::{Entities, Location};
use crate::Entity;

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

pub struct World2 {
    entities: Entities,
    archetypes: ArchetypeSet,
    bundle_to_archetype: TypeIdMap<u32>,
    insert_edges: IndexTypeIdMap<InsertTarget>,
    remove_edges: IndexTypeIdMap<u32>,

    id: AtomicU64,
}
unsafe impl Send for World2 {}
unsafe impl Sync for World2 {}

impl World2 {
    pub fn new() -> Self {
        Self {
            entities: Entities::default(),
            archetypes: ArchetypeSet::new(),
            bundle_to_archetype: TypeIdMap::default(),
            insert_edges: IndexTypeIdMap::default(),
            remove_edges: IndexTypeIdMap::default(),
            id: AtomicU64::new(0),
        }
    }

    pub fn flush(&mut self) {
        let archetype = self.archetypes.get_mut(0);
        self.entities
            .flush(|id, location| location.index = unsafe { archetype.allocate(id) });
    }

    /// Create an entity with certain components
    pub fn spawn(&mut self, components: impl DynamicBundle) -> Entity {
        let entity = self.entities.alloc();
        self.spawn_inner(entity, components);
        entity
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
}

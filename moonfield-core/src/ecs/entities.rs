use std::{
    fmt, mem,
    num::{NonZeroU32, NonZeroU64},
    sync::atomic::{AtomicIsize, Ordering},
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entity {
    pub(crate) generation: NonZeroU32,
    pub(crate) id: u32,
}

impl Entity {
    pub const INVALID: Entity = Entity {
        generation: match NonZeroU32::new(u32::MAX) {
            Some(x) => x,
            None => unreachable!(),
        },
        id: u32::MAX,
    };

    pub const fn to_bits(self) -> NonZeroU64 {
        unsafe {
            NonZeroU64::new_unchecked(
                ((self.generation.get() as u64) << 32) | (self.id as u64),
            )
        }
    }

    pub const fn from_bits(bits: u64) -> Option<Self> {
        Some(Self {
            generation: match NonZeroU32::new((bits >> 32) as u32) {
                Some(g) => g,
                None => return None,
            },
            id: bits as u32,
        })
    }

    pub const fn index(self) -> u32 {
        self.id
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.id, self.generation)
    }
}

pub struct ReservedEntitiesIter<'a> {
    meta: &'a [EntityMetadata],

    id_iter: core::slice::Iter<'a, u32>,

    id_range: core::ops::Range<u32>,
}

impl Iterator for ReservedEntitiesIter<'_> {
    type Item = Entity;

    fn next(&mut self) -> Option<Self::Item> {
        self.id_iter
            .next()
            .map(|&id| Entity {
                generation: self.meta[id as usize].generation,
                id,
            })
            .or_else(|| {
                self.id_range.next().map(|id| Entity {
                    id,
                    generation: NonZeroU32::new(1).unwrap(),
                })
            })
    }
}

pub struct Entities {
    pub meta: Vec<EntityMetadata>,
    /// Entities that have been reserved but not yet inserted into the world
    pending: Vec<u32>,
    /// Index of the first free entity
    /// free_cursor refers the index in pending
    reservation_cursor: AtomicIsize,
    /// Current active entity count
    len: u32,
}

impl Entities {
    pub fn new() -> Self {
        Self {
            meta: Vec::new(),
            pending: Vec::new(),
            reservation_cursor: AtomicIsize::new(0),
            len: 0,
        }
    }

    fn verify_flushed(&mut self) {
        debug_assert!(
            !self.has_reserved_entities(),
            "initialize_reserved_entities() needs to be called before this operation is legal"
        )
    }

    fn has_reserved_entities(&mut self) -> bool {
        *self.reservation_cursor.get_mut() != self.pending.len() as isize
    }

    pub fn alloc(&mut self) -> Entity {
        self.verify_flushed();
        self.len += 1;

        if let Some(id) = self.pending.pop() {
            let new_free_cursor = self.pending.len() as isize;
            *self.reservation_cursor.get_mut() = new_free_cursor;
            Entity { generation: self.meta[id as usize].generation, id }
        } else {
            let id = u32::try_from(self.meta.len()).expect("too many entities");
            self.meta.push(EntityMetadata::EMPTY);
            Entity { generation: NonZeroU32::new(1).unwrap(), id }
        }
    }

    pub fn free(&mut self, entity: Entity) -> Result<Location, NoSuchEntity> {
        self.verify_flushed();

        let meta = self.meta.get_mut(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation != entity.generation
            || meta.location.index == u32::MAX
        {
            return Err(NoSuchEntity);
        }

        meta.generation =
            NonZeroU32::new(u32::from(meta.generation).wrapping_add(1))
                .unwrap_or_else(|| NonZeroU32::new(1).unwrap());

        let loc =
            mem::replace(&mut meta.location, EntityMetadata::EMPTY.location);
        self.pending.push(entity.id);

        let new_free_cursor = self.pending.len() as isize;
        *self.reservation_cursor.get_mut() = new_free_cursor;
        self.len -= 1;

        Ok(loc)
    }

    pub fn reserve_entity(&self) -> Entity {
        // we store the value before atomic subtraction to avoid extra synchronization
        let n = self.reservation_cursor.fetch_sub(1, Ordering::Relaxed);

        if n > 0 {
            let id = self.pending[(n - 1) as usize];
            Entity { generation: self.meta[id as usize].generation, id }
        } else {
            Entity {
                generation: NonZeroU32::new(1).unwrap(),
                id: u32::try_from(self.meta.len() as isize - n)
                    .expect("too many entities"),
            }
        }
    }

    pub fn reserve_entities(&self, count: u32) -> ReservedEntitiesIter<'_> {
        let range_end = self
            .reservation_cursor
            .fetch_sub(count as isize, Ordering::Relaxed);
        let range_start = range_end - count as isize;

        let freelist_range = range_start.max(0) as usize..range_end as usize;

        let (new_id_start, new_id_end) = if range_start >= 0 {
            // If we can use entities in freelist, then we don't need to allocate new entities
            // so we return empty range, then the ReserveEntitiesIterator will use the first element of freelist
            (0, 0)
        } else {
            let base = self.meta.len() as isize;
            // here range_start < 0, so it acutally means that we are calculating the upper bound
            let new_id_end =
                u32::try_from(base - range_start).expect("too many entities");
            let new_id_start = (base - range_end.min(0)) as u32;
            (new_id_start, new_id_end)
        };

        ReservedEntitiesIter {
            meta: &self.meta[..],
            id_iter: self.pending[freelist_range].iter(),
            id_range: new_id_start..new_id_end,
        }
    }

    /// Batch initialize entities
    pub fn initialize_reserved_entities(
        &mut self, mut init: impl FnMut(u32, &mut Location),
    ) {
        let free_cursor = *self.reservation_cursor.get_mut();

        let new_free_cursor = if free_cursor >= 0 {
            free_cursor as usize
        } else {
            // meaning we have more entities than freelist
            let old_meta_len = self.meta.len();
            let new_meta_len = old_meta_len + -free_cursor as usize;
            self.meta.resize(new_meta_len, EntityMetadata::EMPTY);
            self.len += -free_cursor as u32;

            // init new entities
            for (id, meta) in
                self.meta.iter_mut().enumerate().skip(old_meta_len)
            {
                init(id as u32, &mut meta.location);
            }

            *self.reservation_cursor.get_mut() = 0;
            0
        };

        self.len += (self.pending.len() - new_free_cursor) as u32;

        // extract pending entities and init them
        for id in self.pending.drain(new_free_cursor..) {
            init(id, &mut self.meta[id as usize].location);
        }
    }

    #[deprecated(
        since = "0.1.0",
        note = "Renamed to `initialize_reserved_entities` for clarity"
    )]
    pub fn flush(&mut self, init: impl FnMut(u32, &mut Location)) {
        self.initialize_reserved_entities(init)
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn contains(&self, entity: Entity) -> bool {
        // Check if the entity is alive
        if let Some(meta) = self.meta.get(entity.id as usize) {
            meta.generation == entity.generation
                && meta.location.index != u32::MAX
        } else {
            // check if the entity is reserved but not yet allocated
            let free = self.reservation_cursor.load(Ordering::Relaxed);
            entity.generation.get() == 1
                && free < 0
                && (entity.id as isize)
                    < (free.abs() + self.meta.len() as isize)
        }
    }

    pub fn get(&self, entity: Entity) -> Result<Location, NoSuchEntity> {
        let meta = self.meta.get(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation == entity.generation {
            Ok(meta.location)
        } else {
            Err(NoSuchEntity)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Location {
    pub archetype: u32,
    pub index: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EntityMetadata {
    pub generation: NonZeroU32,
    pub location: Location,
}

impl EntityMetadata {
    const EMPTY: EntityMetadata = EntityMetadata {
        generation: match NonZeroU32::new(1) {
            Some(x) => x,
            None => unreachable!(),
        },
        location: Location {
            archetype: 0,
            index: u32::MAX, // dummy value, to be filled in
        },
    };
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoSuchEntity;

impl fmt::Display for NoSuchEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("no such entity")
    }
}
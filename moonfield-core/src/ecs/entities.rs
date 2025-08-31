use std::{
    fmt, mem,
    num::{NonZero, NonZeroU32, NonZeroU64},
    sync::atomic::AtomicIsize,
};

#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entity {
    pub(crate) generation: NonZeroU32,
    pub(crate) id: u32,
}

impl Entity {
    pub const Invalid: Entity = Entity {
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

    pub const fn id(self) -> u32 {
        self.id
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "id: {}, generation: {}", self.id, self.generation)
    }
}

pub struct ReserveEntitiesIterator<'a> {
    meta: &'a [EntityMeta],

    id_iter: core::slice::Iter<'a, u32>,

    id_range: core::ops::Range<u32>,
}

impl Iterator for ReserveEntitiesIterator<'_> {
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
    pub meta: Vec<EntityMeta>,
    /// Entities that have been reserved but not yet inserted into the world
    pending: Vec<u32>,
    /// Index of the first free entity
    free_cursor: AtomicIsize,
    /// Current active entity count
    len: u32,
}

impl Entities {
    pub fn new() -> Self {
        Self {
            meta: Vec::new(),
            pending: Vec::new(),
            free_cursor: AtomicIsize::new(0),
            len: 0,
        }
    }

    fn vertify_flushed(&mut self) {
        debug_assert!(
            !self.needs_flush(),
            "flush() needs to be called before this operation is legal"
        )
    }

    fn needs_flush(&mut self) -> bool {
        *self.free_cursor.get_mut() != self.pending.len() as isize
    }

    pub fn alloc(&mut self) -> Entity {
        self.vertify_flushed();
        self.len += 1;

        if let Some(id) = self.pending.pop() {
            let new_free_cursor = self.pending.len() as isize;
            *self.free_cursor.get_mut() = new_free_cursor;
            Entity { generation: self.meta[id as usize].generation, id }
        } else {
            let id = u32::try_from(self.meta.len()).expect("too many entities");
            self.meta.push(EntityMeta::EMPTY);
            Entity { generation: NonZeroU32::new(1).unwrap(), id }
        }
    }

    pub fn free(&mut self, entity: Entity) -> Result<Location, NoSuchEntity> {
        self.vertify_flushed();

        let meta = self.meta.get_mut(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation != entity.generation
            || meta.location.index == u32::MAX
        {
            return Err(NoSuchEntity);
        }

        meta.generation =
            NonZeroU32::new(u32::from(meta.generation).wrapping_add(1))
                .unwrap_or_else(|| NonZeroU32::new(1).unwrap());

        let loc = mem::replace(&mut meta.location, EntityMeta::EMPTY.location);
        self.pending.push(entity.id);

        let new_free_cursor = self.pending.len() as isize;
        *self.free_cursor.get_mut() = new_free_cursor;
        self.len -= 1;

        Ok(loc)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Location {
    pub archetype: u32,
    pub index: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EntityMeta {
    pub generation: NonZeroU32,
    pub location: Location,
}

impl EntityMeta {
    const EMPTY: EntityMeta = EntityMeta {
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

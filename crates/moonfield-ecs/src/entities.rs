use core::fmt;
use std::{
    error::Error,
    mem,
    num::{NonZeroU32, NonZeroU64},
    ops::Range,
    sync::atomic::{AtomicIsize, Ordering},
    u32,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoSuchEntity;

impl fmt::Display for NoSuchEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("no such entity")
    }
}

impl Error for NoSuchEntity {}

#[derive(Copy, Clone)]
pub(crate) struct Location {
    pub archetype: u32,
    pub index: u32,
}

#[derive(Copy, Clone)]
pub struct EntityMeta {
    pub(crate) generation: NonZeroU32,
    pub(crate) location: Location,
}

impl EntityMeta {
    const EMPTY: EntityMeta = Self {
        generation: match NonZeroU32::new(1) {
            Some(x) => x,
            None => unreachable!(),
        },
        location: Location {
            archetype: 0,
            index: u32::MAX,
        },
    };
}

/// An opaque entity identifier.
///
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    pub(crate) generation: NonZeroU32,
    pub(crate) id: u32,
}

impl Entity {
    pub const DANGLING: Entity = Entity {
        generation: match NonZeroU32::new(u32::MAX) {
            Some(x) => x,
            None => unreachable!(),
        },
        id: u32::MAX,
    };

    pub const fn to_bits(&self) -> NonZeroU64 {
        unsafe {
            NonZeroU64::new_unchecked(((self.generation.get() as u64) << 32) | (self.id as u64))
        }
    }

    pub const fn from_bits(bits: u64) -> Option<Self> {
        Some(Self {
            generation: match NonZeroU32::new((bits >> 32) as u32) {
                Some(x) => x,
                None => return None,
            },
            id: bits as u32,
        })
    }

    pub const fn id(&self) -> u32 {
        self.id
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Entity {{ generation: {}, id: {} }}",
            self.generation.get(),
            self.id
        )
    }
}

pub struct ReserveEntitiesIterator<'a> {
    meta: &'a [EntityMeta],
    // the ids to be used from the free list
    id_iter: core::slice::Iter<'a, u32>,
    // new ids to be used
    id_range: core::ops::Range<u32>,
}

impl Iterator for ReserveEntitiesIterator<'_> {
    type Item = Entity;

    fn next(&mut self) -> Option<Self::Item> {
        self.id_iter
            .next()
            .map(|&id| Entity {
                // use ids from the free list
                generation: self.meta[id as usize].generation,
                id: id,
            })
            .or_else(|| {
                // use new ids
                self.id_range.next().map(|id| Entity {
                    generation: NonZeroU32::MIN,
                    id,
                })
            })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.id_iter.len() + self.id_range.len();
        (len, Some(len))
    }
}

#[derive(Clone)]
pub(crate) struct AllocManyState {
    // the end of kept ids from the pending list
    pub pending_keep_end: usize,
    // if freelist is not enough, then use fresh ids
    fresh: Range<u32>,
}

impl AllocManyState {
    pub fn next(&mut self, entities: &Entities) -> Option<u32> {
        if self.pending_keep_end < entities.pending.len() {
            let id = entities.pending[self.pending_keep_end];
            self.pending_keep_end += 1;
            Some(id)
        } else {
            self.fresh.next()
        }
    }

    pub fn len(&self, entities: &Entities) -> usize {
        self.fresh.len() + (entities.pending.len() - self.pending_keep_end)
    }
}

#[derive(Default)]
pub(crate) struct Entities {
    pub meta: Vec<EntityMeta>,
    // the whole list of entities
    pending: Vec<u32>,
    // indicator for the end of the free list
    free_cursor: AtomicIsize,
    len: u32,
}

impl Entities {
    pub fn reserve_entities(&mut self, count: u32) -> ReserveEntitiesIterator<'_> {
        // range_end is the previous value of free_cursor
        let range_end = self
            .free_cursor
            .fetch_sub(count as isize, Ordering::Relaxed);
        let range_start = range_end - count as isize;

        let freelist_range = range_start.max(0) as usize..range_end.max(0) as usize;

        let (new_id_start, new_id_end) = if range_start > 0 {
            // pending list is sufficient. no need to generate new ids and resize.
            (0, 0)
        } else {
            let base = self.meta.len() as isize;

            // range_start is negative, so we can get now how much more we need
            let new_id_end = u32::try_from(base - range_start).expect("too many entities");

            // if range_end is negative, then we need to generate new ids from the previous len of meta list
            // otherwise, we can use the ids from the free list
            let new_id_start = (base - range_end.min(0)) as u32;
            (new_id_start, new_id_end)
        };
        ReserveEntitiesIterator {
            meta: &self.meta[..],
            id_iter: self.pending[freelist_range].iter(),
            id_range: new_id_start..new_id_end,
        }
    }

    pub fn reserve_entity(&self) -> Entity {
        let n = self.free_cursor.fetch_sub(1, Ordering::Relaxed);
        if n > 0 {
            // use id from free_list
            let id = self.pending[(n - 1) as usize];
            Entity {
                generation: self.meta[id as usize].generation,
                id,
            }
        } else {
            // use new id
            Entity {
                generation: NonZeroU32::MIN,
                id: u32::try_from(self.meta.len() as isize - n).expect("too many entities"),
            }
        }
    }

    pub fn alloc(&mut self) -> Entity {
        // flush will ensure the pending list is all free list. it will drain all reserved entities.
        self.verify_flushed();
        self.len += 1;

        if let Some(id) = self.pending.pop() {
            let new_free_cursor = self.pending.len() as isize;
            *self.free_cursor.get_mut() = new_free_cursor;
            Entity {
                generation: self.meta[id as usize].generation,
                id,
            }
        } else {
            let id = u32::try_from(self.meta.len()).expect("too many entities");
            self.meta.push(EntityMeta::EMPTY);
            Entity {
                generation: NonZeroU32::MIN,
                id,
            }
        }
    }

    pub fn alloc_many(&mut self, n: u32, archetype: u32, mut first_index: u32) -> AllocManyState {
        self.verify_flushed();

        let fresh_needed = (n as usize).saturating_sub(self.pending.len()) as u32;
        assert!(
            (self.meta.len() + fresh_needed as usize) < u32::MAX as usize,
            "too many entities"
        );

        let pending_keep_end = self.pending.len().saturating_sub(n as usize);
        for &id in &self.pending[pending_keep_end..] {
            self.meta[id as usize].location = Location {
                archetype,
                index: first_index,
            };
            first_index += 1;
        }

        let fresh_start = self.meta.len() as u32;
        self.meta.extend(
            (first_index..(first_index + fresh_needed)).map(|index| EntityMeta {
                generation: NonZeroU32::MIN,
                location: Location { archetype, index },
            }),
        );

        self.len += n;

        AllocManyState {
            pending_keep_end,
            fresh: fresh_start..(fresh_start + fresh_needed),
        }
    }

    pub fn finish_alloc_many(&mut self, pending_keep_end: usize) {
        self.pending.truncate(pending_keep_end);
        *self.free_cursor.get_mut() = pending_keep_end as isize;
    }

    pub fn alloc_at(&mut self, entity: Entity) -> Option<Location> {
        self.verify_flushed();

        let loc = if entity.id as usize >= self.meta.len() {
            // ID was never used. We need to resize the meta list and fill the gap between the last used id and the new id.
            self.pending.extend(self.meta.len() as u32..entity.id);
            let new_free_cursor = self.pending.len() as isize;
            *self.free_cursor.get_mut() = new_free_cursor;
            self.meta.resize(entity.id as usize + 1, EntityMeta::EMPTY);
            self.len += 1;
            None
        } else if let Some(index) = self.pending.iter().position(|item| *item == entity.id) {
            // ID is previously used, but it is in the free list. We need to swap it out.
            self.pending.swap_remove(index);
            let new_free_cursor = self.pending.len() as isize;
            *self.free_cursor.get_mut() = new_free_cursor;
            self.len += 1;
            None
        } else {
            // ID is currently used, so we need to replace the location.
            Some(mem::replace(
                &mut self.meta[entity.id as usize].location,
                EntityMeta::EMPTY.location,
            ))
        };

        loc
    }

    fn needs_flush(&mut self) -> bool {
        *self.free_cursor.get_mut() != self.pending.len() as isize
    }

    fn verify_flushed(&mut self) {
        debug_assert!(
            !self.needs_flush(),
            "flush() needs to be called before this operation is legal"
        );
    }

    pub fn flush(&mut self, mut init: impl FnMut(u32, &mut Location)) {}

    pub fn free(&mut self, entity: Entity) -> Result<Location, NoSuchEntity> {
        self.verify_flushed();

        let meta = self.meta.get_mut(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation != entity.generation || meta.location.index == u32::MAX {
            return Err(NoSuchEntity);
        }

        meta.generation = NonZeroU32::new(u32::from(meta.generation).wrapping_add(1))
            .unwrap_or_else(|| NonZeroU32::MIN);

        let loc = mem::replace(&mut meta.location, EntityMeta::EMPTY.location);

        self.pending.push(entity.id);

        let new_free_cursor = self.pending.len() as isize;
        *self.free_cursor.get_mut() = new_free_cursor;
        self.len -= 1;

        Ok(loc)
    }

    pub fn reserve(&mut self, additional: u32) {
        self.verify_flushed();

        let freelist_size = *self.free_cursor.get_mut();
        let shortfall = additional as isize - freelist_size;
        if shortfall > 0 {
            self.meta.reserve(shortfall as usize);
        }
    }
}

/// Internal generation-tracking storage.  In a minimal ECS we only track the
/// next free id and the set of alive ids so that `despawn` works.
#[derive(Default, Debug)]
pub(crate) struct EntityId {
    next: u64,
    alive: std::collections::HashSet<u64>,
}

impl EntityId {
    pub fn alloc(&mut self) -> Entity {
        let id = self.next;
        self.next += 1;
        self.alive.insert(id);
        Entity::from_bits(id).unwrap()
    }

    pub fn free(&mut self, entity: Entity) -> bool {
        self.alive.remove(&entity.to_bits().get())
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.alive.contains(&entity.to_bits().get())
    }

    pub fn alive_count(&self) -> usize {
        self.alive.len()
    }

    pub fn alive_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive.iter().copied().filter_map(Entity::from_bits)
    }
}

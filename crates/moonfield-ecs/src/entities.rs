use core::fmt;
use std::{
    cmp,
    error::Error,
    mem,
    num::NonZeroU32,
    num::NonZeroU64,
    ops::Range,
    sync::atomic::{AtomicIsize, Ordering},
};

/// An error type for when an entity with a particular ID does not exist.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NoSuchEntity;

impl fmt::Display for NoSuchEntity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("no such entity")
    }
}

impl Error for NoSuchEntity {}

/// The location of an entity.
///
/// This struct is used to locate the entity in the world.
#[derive(Copy, Clone)]
pub(crate) struct Location {
    /// The archetype that the entity belongs to. World will maintain a list of all archetypes.
    pub archetype: u32,
    /// The index of the entity in the archetype. Archetype will maintain a list of all entities in the archetype.
    pub index: u32,
}

/// Entity properties.
///
/// This struct is used to store the properties of an entity, such as its generation and location.
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
/// When packed into a `u64` via [`to_bits`](Entity::to_bits), the layout is:
///
/// ```text
///  63      32 | 31       0
///  ───────────┼───────────
///  generation │    id
///  ───────────┼───────────
///   high 32   │  low 32
/// ```
///
/// - **Bits 63..32 (high 32 bits)** — `generation`: incremented each time the entity
///   is freed, used to detect dangling references.
/// - **Bits 31..0  (low 32 bits)** — `id`: index into the global [`Entities`] metadata
///   array.
///
/// See also [`to_bits`](Entity::to_bits) and [`from_bits`](Entity::from_bits).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    /// The generation of the entity, incremented each time the entity is freed.
    /// Packed into the high 32 bits of the `u64` representation.
    pub(crate) generation: NonZeroU32,
    /// The unique index of the entity in the global [`Entities`] metadata array.
    /// Packed into the low 32 bits of the `u64` representation.
    pub(crate) id: u32,
}

impl Entity {
    /// A sentinel entity that is guaranteed to never be a valid entity.
    ///
    /// Both `generation` (`NonZeroU32(u32::MAX)`) and `id` (`u32::MAX`) are set to
    /// their maximum possible values, making it impossible for this to match any
    /// real entity returned by the allocator. Useful as a placeholder or default
    /// value when an [`Entity`] is required but no valid entity exists.
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
                id,
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

/// Track the state of allocation of many entities.
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

/// The central entity allocator and metadata tracker.
///
/// Manages all entity IDs, their generations, and their locations within
/// archetypes.  The design uses a **single `pending` list** partitioned by
/// `free_cursor` to serve as both the free-list and the reservation buffer,
/// avoiding a separate allocation structure.
///
/// # Layout
///
/// ```text
///                        ┌── free_cursor ──┐
///                        │                 │
/// pending = [  freed IDs  |  reserved IDs  ]
///            ◄────────────►◄──────────────►
///              free list      reservation
/// ```
///
/// - `pending[..free_cursor]` — IDs that have been freed and are ready to
///   reuse (the **free list**).
/// - `pending[free_cursor..]` — IDs that have been reserved via
///   [`reserve_entities`](Entities::reserve_entities) but not yet flushed.
///
/// When `free_cursor` is **negative**, its absolute value represents the
/// number of IDs that must be allocated beyond the current free list — i.e.
/// fresh IDs from the [`meta`](Self::meta) extension.
#[derive(Default)]
pub(crate) struct Entities {
    /// Metadata array indexed by entity ID.
    ///
    /// `meta[id]` holds the [`EntityMeta`] (generation + location) for the
    /// entity with that ID.  The array grows monotonically — its length is
    /// always `max_ever_allocated_id + 1`.  Freed entities keep their slot
    /// so that dangling references can be detected via generation mismatch.
    pub meta: Vec<EntityMeta>,

    /// A dual-purpose list of entity IDs.
    ///
    /// The first `free_cursor` entries (when `free_cursor >= 0`) form the
    /// **free list** — IDs of previously freed entities that can be reused.
    /// The remaining entries are **reserved IDs** — IDs that have been
    /// claimed by [`reserve_entities`](Entities::reserve_entities) but not
    /// yet finalized via [`flush`](Entities::flush).
    pending: Vec<u32>,

    /// Boundary pointer that partitions `pending` into the free list and
    /// the reservation buffer (see the struct-level docs for details).
    ///
    /// - `>= 0` — number of free-list entries at the front of `pending`.
    /// - `< 0` — the free list is exhausted; `-free_cursor` extra IDs must
    ///   come from extending [`meta`](Self::meta).
    free_cursor: AtomicIsize,

    /// Number of currently alive (allocated) entities.
    len: u32,
}

impl Entities {
    #[inline]
    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn freelist(&self) -> impl ExactSizeIterator<Item = Entity> + '_ {
        let free = self.free_cursor.load(Ordering::Relaxed);
        let ids = match usize::try_from(free) {
            Err(_) => &[],
            Ok(free) => &self.pending[0..free],
        };
        ids.iter().map(|&id| Entity {
            id,
            generation: self.meta[id as usize].generation,
        })
    }

    pub fn set_freelist(&mut self, freelist: &[Entity]) {
        #[cfg(debug_assertions)]
        {
            for entity in freelist {
                let Some(meta) = self.meta.get(entity.id as usize) else {
                    continue;
                };
                // check if the entity is alive, if is, then we cannot set it to the free list
                assert_eq!(
                    meta.location.index,
                    u32::MAX,
                    "freelist addresses live entities"
                );
            }
        }
        if let Some(max) = freelist.iter().map(|e: &Entity| e.id()).max() {
            // If some id is bigger than the lenth of meta, We will resize the meta list
            if max as usize >= self.meta.len() {
                self.meta.resize(max as usize + 1, EntityMeta::EMPTY);
            }
        }
        // Reconstruct pending list
        self.pending.clear();
        for entity in freelist {
            self.pending.push(entity.id);
            self.meta[entity.id as usize].generation = entity.generation;
        }
        self.free_cursor = AtomicIsize::new(freelist.len() as isize);
    }

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

    pub fn flush(&mut self, mut init: impl FnMut(u32, &mut Location)) {
        let free_cursor = *self.free_cursor.get_mut();

        let new_free_cursor = if free_cursor >= 0 {
            free_cursor as usize
        } else {
            // If the free cursor is negative, then we need to resize the meta list
            let old_meta_len = self.meta.len();
            let new_meta_len = old_meta_len + -free_cursor as usize;
            self.meta.resize(new_meta_len, EntityMeta::EMPTY);
            self.len += -free_cursor as u32;
            for (id, meta) in self.meta.iter_mut().enumerate().skip(old_meta_len) {
                init(id as u32, &mut meta.location);
            }

            *self.free_cursor.get_mut() = 0;
            0
        };

        self.len += (self.pending.len() - new_free_cursor) as u32;
        // initialize the remaining pending list
        for id in self.pending.drain(new_free_cursor..) {
            init(id, &mut self.meta[id as usize].location);
        }
    }

    pub fn free(&mut self, entity: Entity) -> Result<Location, NoSuchEntity> {
        self.verify_flushed();

        let meta = self.meta.get_mut(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation != entity.generation || meta.location.index == u32::MAX {
            return Err(NoSuchEntity);
        }

        meta.generation =
            NonZeroU32::new(u32::from(meta.generation).wrapping_add(1)).unwrap_or(NonZeroU32::MIN);

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

    pub fn contains(&self, entity: Entity) -> bool {
        match self.meta.get(entity.id as usize) {
            Some(meta) => {
                meta.generation == entity.generation && (meta.location.index != u32::MAX)
                    || self.pending[self.free_cursor.load(Ordering::Relaxed).max(0) as usize..]
                        .contains(&entity.id)
            }
            None => {
                let free = self.free_cursor.load(Ordering::Relaxed);
                entity.generation.get() == 1
                    && free < 0
                    && (entity.id as isize) < (free.abs() + self.meta.len() as isize)
            }
        }
    }

    pub fn clear(&mut self) {
        self.meta.clear();
        self.pending.clear();
        *self.free_cursor.get_mut() = 0;
        self.len = 0;
    }

    pub fn get_mut(&mut self, entity: Entity) -> Result<&mut Location, NoSuchEntity> {
        let meta = self.meta.get_mut(entity.id as usize).ok_or(NoSuchEntity)?;
        if meta.generation == entity.generation && meta.location.index != u32::MAX {
            Ok(&mut meta.location)
        } else {
            Err(NoSuchEntity)
        }
    }

    pub fn get(&self, entity: Entity) -> Result<Location, NoSuchEntity> {
        if self.meta.len() <= entity.id as usize {
            // Check if this could have been obtained from `reserve_entity`
            let free = self.free_cursor.load(Ordering::Relaxed);
            if entity.generation.get() == 1
                && free < 0
                && (entity.id as isize) < (free.abs() + self.meta.len() as isize)
            {
                return Ok(Location {
                    archetype: 0,
                    index: u32::MAX,
                });
            } else {
                return Err(NoSuchEntity);
            }
        }
        let meta = &self.meta[entity.id as usize];
        if meta.generation != entity.generation || meta.location.index == u32::MAX {
            return Err(NoSuchEntity);
        }
        Ok(meta.location)
    }

    pub unsafe fn resolve_unknown_gen(&self, id: u32) -> Entity {
        let meta_len = self.meta.len();

        if meta_len > id as usize {
            let meta = &self.meta[id as usize];
            Entity {
                generation: meta.generation,
                id,
            }
        } else {
            // See if it's pending, but not yet flushed.
            let free_cursor = self.free_cursor.load(Ordering::Relaxed);
            let num_pending = cmp::max(-free_cursor, 0) as usize;

            if meta_len + num_pending > id as usize {
                // Pending entities will have the first generation.
                Entity {
                    generation: NonZeroU32::MIN,
                    id,
                }
            } else {
                panic!("entity id is out of range");
            }
        }
    }
}

/// Internal generation-tracking storage.  In a minimal ECS we only track the
/// next free id and the set of alive ids so that `despawn` works.
///
/// Entities are created with generation 1 and stored in `alive` as packed
/// `Entity::to_bits()`, so all lookups (`free`, `is_alive`, `alive_entities`)
/// use the same encoding.
#[derive(Default, Debug)]
pub(crate) struct EntityId {
    next: u32,
    alive: std::collections::HashSet<u64>,
}

impl EntityId {
    pub fn alloc(&mut self) -> Entity {
        let id = self.next;
        self.next += 1;
        let entity = Entity {
            generation: NonZeroU32::MIN,
            id,
        };
        self.alive.insert(entity.to_bits().get());
        entity
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

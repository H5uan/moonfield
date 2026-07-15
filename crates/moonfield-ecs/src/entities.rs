use core::fmt;
use std::{
    error::Error,
    num::{NonZeroU32, NonZeroU64},
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

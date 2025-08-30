use std::{
    fmt,
    num::{NonZero, NonZeroU32, NonZeroU64},
};

pub struct Entity {
    pub(crate) id: u32,
    pub(crate) generation: NonZeroU32,
}

impl Entity {
    pub const DANGLING: Entity = Entity {
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
            id: bits as u32,
            generation: match NonZeroU32::new((bits >> 32) as u32) {
                Some(g) => g,
                None => return None,
            },
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

pub struct EntityMeta {}

pub struct Entities {
    pub meta: Vec<EntityMeta>,
}

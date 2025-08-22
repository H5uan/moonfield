use std::num::NonZeroU32;



pub struct Entity {
    pub(crate) id: u32,
    pub(crate) generation: u32,
}

impl Entity {
    
}

pub struct EntityMeta {}


pub struct Entities {
    pub meta: Vec<EntityMeta>,
}
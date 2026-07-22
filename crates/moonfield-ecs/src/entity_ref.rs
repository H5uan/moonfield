use crate::{Component, Entity, archetype::Archetype, entities::EntityMeta};

#[derive(Copy, Clone)]
pub struct EntityRef<'a> {
    meta: &'a [EntityMeta],
    archetype: &'a Archetype,
    row_index: u32,
}

impl<'a> EntityRef<'a> {
    pub(crate) unsafe fn new(
        meta: &'a [EntityMeta],
        archetype: &'a Archetype,
        row_index: u32,
    ) -> Self {
        Self {
            meta,
            archetype,
            row_index,
        }
    }

    #[inline]
    pub fn entity(&self) -> Entity {
        let id = self.archetype.entity_id(self.row_index);
        Entity {
            id,
            generation: self.meta[id as usize].generation,
        }
    }

    pub fn has<T: Component>(&self) -> bool {
        self.archetype.has::<T>()
    }
}

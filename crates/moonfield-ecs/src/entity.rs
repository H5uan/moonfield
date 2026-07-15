/// An opaque entity identifier.
///
/// Bevy uses a packed (index, generation) pair; here we keep it simple
/// with a single `u64` so that equality is cheap and copies are trivial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity(u64);

impl Entity {
    pub(crate) fn from_raw(id: u64) -> Self {
        Self(id)
    }

    /// Raw underlying value, mostly useful for debugging.
    pub fn id(&self) -> u64 {
        self.0
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
        Entity(id)
    }

    pub fn free(&mut self, entity: Entity) -> bool {
        self.alive.remove(&entity.0)
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.alive.contains(&entity.0)
    }

    pub fn alive_count(&self) -> usize {
        self.alive.len()
    }

    pub fn alive_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive.iter().copied().map(Entity::from_raw)
    }
}

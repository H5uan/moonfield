//! Minimal synchronous asset storage for moonfield: [`Assets<T>`] typed
//! stores (one world resource per asset type) and [`Handle<T>`] typed
//! references into them.
//!
//! Deliberately tiny (Bevy's `bevy_asset` shape, minus the machinery):
//!
//! - **Synchronous only.** The caller loads bytes and inserts the asset
//!   ([`Assets::add`]) itself; there is no `AssetServer`, no task pool, no
//!   async. Those are roadmap known-debts.
//! - **Handles are plain ids** (index + generation), not reference counted.
//!   [`Assets::remove`] invalidates a handle by bumping the slot's
//!   generation; a stale handle simply resolves to `None`
//!   ([`Assets::get`]). Nothing tracks "last strong handle dropped".
//!
//! This crate has no ECS dependency: `Handle<T>` and `Assets<T>` are
//! components/resources through the blanket `Component`/`Resource` impls in
//! `moonfield-ecs`.

use std::fmt;
use std::marker::PhantomData;

/// Index + generation identifier of an asset inside an [`Assets<T>`] store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId {
    index: u32,
    generation: u32,
}

/// A typed reference to an asset stored in [`Assets<T>`].
///
/// `Copy` and `Send + Sync` regardless of `T` (the `PhantomData<fn() -> T>`
/// marker only carries type information). Cheap to store on entities as a
/// component.
pub struct Handle<T: 'static> {
    id: AssetId,
    marker: PhantomData<fn() -> T>,
}

impl<T: 'static> Handle<T> {
    fn new(id: AssetId) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    /// The underlying [`AssetId`].
    pub fn id(&self) -> AssetId {
        self.id
    }
}

// Manual trait impls so `Handle<T>` stays `Copy`/`Clone`/`Eq`/`Hash` even
// when `T` is none of those.
impl<T: 'static> Copy for Handle<T> {}
impl<T: 'static> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T: 'static> Eq for Handle<T> {}
impl<T: 'static> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl<T: 'static> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Handle<{}>({}v{})",
            std::any::type_name::<T>(),
            self.id.index,
            self.id.generation
        )
    }
}

struct Slot<T> {
    generation: u32,
    asset: Option<T>,
}

/// A typed asset store: one dense slot map per asset type.
///
/// Stored as a world resource (one per asset type, e.g.
/// `Assets<SplatCloud>`). Freed slots are reused with a bumped generation,
/// so stale handles to removed assets resolve to `None` instead of aliasing
/// a newer asset.
pub struct Assets<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }
}

impl<T: 'static> Assets<T> {
    /// Insert an asset, returning its handle. Reuses a freed slot if one is
    /// available (with a fresh generation).
    pub fn add(&mut self, asset: T) -> Handle<T> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            debug_assert!(slot.asset.is_none());
            slot.asset = Some(asset);
            self.len += 1;
            Handle::new(AssetId {
                index,
                generation: slot.generation,
            })
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot {
                generation: 0,
                asset: Some(asset),
            });
            self.len += 1;
            Handle::new(AssetId {
                index,
                generation: 0,
            })
        }
    }

    /// Resolve a handle. Returns `None` for removed or foreign handles.
    pub fn get(&self, handle: &Handle<T>) -> Option<&T> {
        let slot = self.slots.get(handle.id.index as usize)?;
        if slot.generation != handle.id.generation {
            return None;
        }
        slot.asset.as_ref()
    }

    /// Resolve a handle mutably. Returns `None` for removed or foreign
    /// handles.
    pub fn get_mut(&mut self, handle: &Handle<T>) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.id.index as usize)?;
        if slot.generation != handle.id.generation {
            return None;
        }
        slot.asset.as_mut()
    }

    /// Whether `handle` currently resolves to a live asset.
    pub fn contains(&self, handle: &Handle<T>) -> bool {
        self.get(handle).is_some()
    }

    /// Remove the asset a handle points to, returning it. The handle (and
    /// any copies of it) is stale afterwards; the slot may later be reused
    /// by [`add`](Self::add) with a new generation.
    pub fn remove(&mut self, handle: &Handle<T>) -> Option<T> {
        let slot = self.slots.get_mut(handle.id.index as usize)?;
        if slot.generation != handle.id.generation {
            return None;
        }
        let asset = slot.asset.take()?;
        slot.generation += 1;
        self.free.push(handle.id.index);
        self.len -= 1;
        Some(asset)
    }

    /// Iterate `(handle, asset)` pairs for all live assets.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.asset.as_ref().map(|asset| {
                (
                    Handle::new(AssetId {
                        index: index as u32,
                        generation: slot.generation,
                    }),
                    asset,
                )
            })
        })
    }

    /// Number of live assets.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the store holds no assets.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_get_roundtrip() {
        let mut assets = Assets::<String>::default();
        let a = assets.add("a".to_string());
        let b = assets.add("b".to_string());
        assert_eq!(assets.get(&a).map(String::as_str), Some("a"));
        assert_eq!(assets.get(&b).map(String::as_str), Some("b"));
        assert_ne!(a, b);
        assert_eq!(assets.len(), 2);
    }

    #[test]
    fn test_get_mut_edits_in_place() {
        let mut assets = Assets::<u32>::default();
        let h = assets.add(1);
        *assets.get_mut(&h).unwrap() = 42;
        assert_eq!(assets.get(&h), Some(&42));
    }

    #[test]
    fn test_remove_invalidates_handle() {
        let mut assets = Assets::<u32>::default();
        let h = assets.add(7);
        assert_eq!(assets.remove(&h), Some(7));
        assert!(!assets.contains(&h));
        assert!(assets.get(&h).is_none());
        // Double remove is a no-op.
        assert!(assets.remove(&h).is_none());
        assert!(assets.is_empty());
    }

    #[test]
    fn test_freed_slot_reuse_bumps_generation() {
        let mut assets = Assets::<u32>::default();
        let old = assets.add(1);
        assets.remove(&old);

        let new = assets.add(2);
        // Same slot index, fresh generation: the stale handle must not
        // resolve to the new asset.
        assert_eq!(old.id().index, new.id().index);
        assert_ne!(old, new);
        assert!(assets.get(&old).is_none());
        assert_eq!(assets.get(&new), Some(&2));
    }

    #[test]
    fn test_iter_yields_live_assets() {
        let mut assets = Assets::<u32>::default();
        let a = assets.add(1);
        let b = assets.add(2);
        let c = assets.add(3);
        assets.remove(&b);

        let live: Vec<(Handle<u32>, u32)> = assets.iter().map(|(h, v)| (h, *v)).collect();
        assert_eq!(live, vec![(a, 1), (c, 3)]);
    }

    #[test]
    fn test_handle_is_copy_regardless_of_payload() {
        struct NotCopy(String);
        let mut assets = Assets::<NotCopy>::default();
        let h = assets.add(NotCopy("x".into()));
        let copied = h; // Handle<T> is Copy even though NotCopy is not
        assert_eq!(h, copied);
        assert!(assets.contains(&copied));
        assert_eq!(assets.get(&h).map(|n| n.0.as_str()), Some("x"));
    }

    #[test]
    fn test_assets_is_a_world_resource() {
        // Blanket Resource impl in moonfield-ecs covers Assets<T>.
        let mut world = moonfield_ecs::World::new();
        world.insert_resource(Assets::<u32>::default());
        let handle = world.get_resource_mut::<Assets<u32>>().unwrap().add(5);
        assert_eq!(
            world.get_resource::<Assets<u32>>().unwrap().get(&handle),
            Some(&5)
        );
    }
}

use crate::{Component, ComponentStorage, Entity, World};

/// Trait for types that can be fetched from a [`World`] query.
///
/// Implemented for:
/// - `&T` – immutable component access
/// - `&mut T` – mutable component access
/// - Tuples `(A, B)` and `(A, B, C)` – joined multi-component queries
/// - `Option<&T>` – optional component access
/// - `Entity` – entity ids
pub trait Query {
    type Item<'w>: 'w
    where
        Self: 'w;
    type Iter<'w>: Iterator<Item = (Entity, Self::Item<'w>)>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w;
    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w;
}

// ------------------------------------------------------------------
// Immutable single component query: &T
// ------------------------------------------------------------------

impl<T: Component> Query for &T {
    type Item<'w> = &'w T
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, &'w T)> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        match world.component_storage::<T>() {
            Some(storage) => Box::new(storage.iter()),
            None => Box::new(std::iter::empty()),
        }
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
    }
}

// ------------------------------------------------------------------
// Mutable single component query: &mut T
// ------------------------------------------------------------------

impl<T: Component> Query for &mut T {
    type Item<'w> = &'w mut T
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, &'w mut T)> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(_world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Box::new(std::iter::empty())
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        match world.component_storage_mut::<T>() {
            Some(storage) => Box::new(storage.iter_mut()),
            None => Box::new(std::iter::empty()),
        }
    }
}

// ------------------------------------------------------------------
// Two-component tuple queries (immutable, immutable)
// ------------------------------------------------------------------

impl<A: Component, B: Component> Query for (&A, &B) {
    type Item<'w> = (&'w A, &'w B)
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, (&'w A, &'w B))> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        match (world.component_storage::<A>(), world.component_storage::<B>()) {
            (Some(a), Some(b)) => {
                if a.len() <= b.len() {
                    Box::new(a.iter().filter_map(move |(e, a_val)| {
                        b.get(e).map(|b_val| (e, (a_val, b_val)))
                    }))
                } else {
                    Box::new(b.iter().filter_map(move |(e, b_val)| {
                        a.get(e).map(|a_val| (e, (a_val, b_val)))
                    }))
                }
            }
            _ => Box::new(std::iter::empty()),
        }
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
    }
}

// ------------------------------------------------------------------
// Two-component tuple queries (mutable, immutable)
// ------------------------------------------------------------------

impl<A: Component, B: Component> Query for (&mut A, &B) {
    type Item<'w> = (&'w mut A, &'w B)
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, (&'w mut A, &'w B))> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(_world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Box::new(std::iter::empty())
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        // SAFETY: A and B are disjoint component storages.
        let world_ptr = world as *mut World;
        let a_storage = unsafe { (*world_ptr).component_storage_mut::<A>() };
        let b_storage = unsafe { (*world_ptr).component_storage::<B>() };
        match (a_storage, b_storage) {
            (Some(a), Some(b)) => {
                let b_ptr = b as *const ComponentStorage<B>;
                Box::new(a.iter_mut().filter_map(move |(e, a_val)| {
                    let b_val = unsafe { (*b_ptr).get(e) }?;
                    Some((e, (a_val, b_val)))
                }))
            }
            _ => Box::new(std::iter::empty()),
        }
    }
}

// ------------------------------------------------------------------
// Two-component tuple queries (mutable, mutable)
// ------------------------------------------------------------------

impl<A: Component, B: Component> Query for (&mut A, &mut B) {
    type Item<'w> = (&'w mut A, &'w mut B)
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, (&'w mut A, &'w mut B))> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(_world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Box::new(std::iter::empty())
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        // SAFETY: A and B are disjoint component storages.
        let world_ptr = world as *mut World;
        let a_storage = unsafe { (*world_ptr).component_storage_mut::<A>() };
        let b_storage = unsafe { (*world_ptr).component_storage_mut::<B>() };
        match (a_storage, b_storage) {
            (Some(a), Some(b)) => {
                let b_ptr = b as *mut ComponentStorage<B>;
                Box::new(a.iter_mut().filter_map(move |(e, a_val)| {
                    let b_val = unsafe { (*b_ptr).get_mut(e) }?;
                    Some((e, (a_val, b_val)))
                }))
            }
            _ => Box::new(std::iter::empty()),
        }
    }
}

// ------------------------------------------------------------------
// Three-component tuple queries (immutable, immutable, immutable)
// ------------------------------------------------------------------

impl<A: Component, B: Component, C: Component> Query for (&A, &B, &C) {
    type Item<'w> = (&'w A, &'w B, &'w C)
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, (&'w A, &'w B, &'w C))> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        match (world.component_storage::<A>(), world.component_storage::<B>(), world.component_storage::<C>()) {
            (Some(a), Some(b), Some(c)) => {
                Box::new(a.iter().filter_map(move |(e, a_val)| {
                    b.get(e).and_then(|b_val| c.get(e).map(|c_val| (e, (a_val, b_val, c_val))))
                }))
            }
            _ => Box::new(std::iter::empty()),
        }
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
    }
}

// ------------------------------------------------------------------
// Optional component query
// ------------------------------------------------------------------

impl<T: Component> Query for Option<&T> {
    type Item<'w> = Option<&'w T>
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, Option<&'w T>)> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        match world.component_storage::<T>() {
            Some(storage) => Box::new(storage.iter().map(|(e, c)| (e, Some(c)))),
            None => Box::new(std::iter::empty()),
        }
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
    }
}

// ------------------------------------------------------------------
// Entity id query
// ------------------------------------------------------------------

impl Query for Entity {
    type Item<'w> = Entity
    where
        Self: 'w;
    type Iter<'w> = Box<dyn Iterator<Item = (Entity, Entity)> + 'w>
    where
        Self: 'w;

    fn fetch<'w>(world: &'w World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Box::new(world.entities().alive_entities().map(|e| (e, e)))
    }

    fn fetch_mut<'w>(world: &'w mut World) -> Self::Iter<'w>
    where
        Self: 'w,
    {
        Self::fetch(world)
    }
}

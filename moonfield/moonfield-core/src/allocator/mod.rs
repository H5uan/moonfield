pub mod handle;
pub mod slot;

use std::{cell::UnsafeCell, marker::PhantomData, thread::panicking};

pub use handle::*;
pub use slot::*;

const INVALID_INDEX: u32 = 0;
const INVALID_GENERATION: u32 = 0;

/// Pool is a wrapper for continuous array
/// free_stack is for tracking free slot
#[derive(Debug)]
pub struct Pool<T, S = Option<T>>
where
    T: Sized,
    S: Slot<Element = T>,
{
    records: Vec<PoolRecord<T, S>>,
    free_stack: Vec<u32>,
}

impl<T, S> PartialEq for Pool<T, S>
where
    T: PartialEq,
    S: Slot<Element = T> + PartialEq,
{
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.records == other.records
    }
}

impl<T, S> Default for Pool<T, S>
where
    T: 'static,
    S: Slot<Element = T> + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, S> Clone for Pool<T, S>
where
    T: Clone,
    S: Slot<Element = T> + Clone + 'static,
{
    #[inline]
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            free_stack: self.free_stack.clone(),
        }
    }
}

impl<T, S> Pool<T, S>
where
    S: Slot<Element = T> + 'static,
{
    #[inline]
    pub fn new() -> Self {
        Pool {
            records: Vec::new(),
            free_stack: Vec::new(),
        }
    }

    #[inline]
    pub fn with_capacity(capacity: u32) -> Self {
        let capacity = usize::try_from(capacity).expect("capacity overflowed usize");
        Self {
            records: Vec::with_capacity(capacity),
            free_stack: Vec::new(),
        }
    }

    fn records_len(&self) -> u32 {
        u32::try_from(self.records.len()).expect("Number of recors overflowed u32")
    }

    fn records_get(&self, index: u32) -> Option<&PoolRecord<T, S>> {
        let index = usize::try_from(index).expect("Index overflowed usize");
        self.records.get(index)
    }

    fn records_get_mut(&mut self, index: u32) -> Option<&mut PoolRecord<T, S>> {
        let index = usize::try_from(index).expect("Index overflowed usize");
        self.records.get_mut(index)
    }

    pub fn spawn(&mut self, slot: T) -> Handle<T> {
        self.spawn_with(|_| slot)
    }

    /// spawn_with allows you to get handle when it created. It will avoid finding
    ///
    /// Node-based data structure needs to know its handle to create link
    ///
    /// instead of seperately create object and handle, we use callback to create
    /// handle when we create object
    /// ```
    /// // 1. allocate memory
    /// let handle = pool.allocate();
    /// // 2. create object but do not know its memory handle
    /// let object = MyObject::new();
    ///
    /// pool.store(handle, object);

    /// ```
    ///
    /// ```
    /// let handle = pool.spawn_with(|handle| {MyObject::new(handle)});

    /// ```
    pub fn spawn_with<F: FnOnce(Handle<T>) -> T>(&mut self, callback: F) -> Handle<T> {
        if let Some(free_index) = self.free_stack.pop() {
            let record = self
                .records_get_mut(free_index)
                .expect("free stack contained invalid index");

            if record.slot.is_some() {
                std::panic!(
                    "Attempt to spawn an object to pool record with slot! Record index is {free_index}"
                );
            }

            let generation = record.generation + 1;
            let handle = Handle {
                index: free_index,
                generation,
                type_marker: PhantomData,
            };
            let slot = callback(handle);

            record.generation = generation;
            record.slot.replace(slot);

            handle
        } else {
            // no free records, create a new record
            let generation = 1;
            let index = self.records_len();

            let handle = Handle {
                index,
                generation,
                type_marker: PhantomData,
            };

            let slot = callback(handle);

            let record = PoolRecord {
                ref_counter: Default::default(),
                generation: 1,
                slot: SlotWrapper::new(slot),
            };

            self.records.push(record);

            handle
        }
    }

    /// Moves object out of the pool using the given handle. All handles to the object will become invalid.
    ///
    /// # Panics
    ///
    /// Panics if the given handle is invalid.
    pub fn free(&mut self, handle: Handle<T>) -> T {
        let index = usize::try_from(handle.index).expect("index overflowed usize");

        if index >= self.records.len() {
            panic!(
                "Attempt to free destroyed object using out-of-bounds handle {:?}! Record count is {}",
                handle,
                self.records.len()
            )
        }

        let record = &mut self.records[index];
        if record.generation != handle.generation {
            panic!(
                "Attempt to free object using dangling handle {:?}! Record generation is {}",
                handle, record.generation
            );
        }

        // extract the slot and set the origin slot as None
        // after take, the mutable reference of record will end immediately
        if let Some(slot) = record.slot.take() {
            self.free_stack.push(handle.index);
            slot
        } else {
            panic!("Attempt to double free object at handle {handle:?}!")
        }
    }

    pub fn borrow(&self, handle: Handle<T>) -> &T {
        // store the records length on stack can let it not depend on borrowing
        let records_len = self.records.len();

        if let Some(record) = self.records_get(handle.index) {
            if record.generation == handle.generation {
                if let Some(slot) = record.slot.as_ref() {
                    slot
                } else {
                    panic!("Attempt to borrow destroyed object at {handle:?} handle.")
                }
            } else {
                panic!(
                    "Attempt to use dangling handle {:?}. Record has generation {}!",
                    handle, record.generation
                );
            }
        } else {
            panic!(
                "Attempt to borrow object using out-of-bounds handle {:?}!, Record count is {}",
                handle, records_len
            );
        }
    }

    pub fn borrow_mut(&mut self, handle: Handle<T>) -> &mut T {
        let records_len = self.records.len();

        if let Some(record) = self.records_get_mut(handle.index) {
            if record.generation == handle.generation {
                if let Some(slot) = record.slot.as_mut() {
                    slot
                } else {
                    panic!("Attempt to borrow destroyed object at {handle:?} handle.")
                }
            } else {
                panic!(
                    "Attempt to use dangling handle {:?}. Record has generation {}!",
                    handle, record.generation
                );
            }
        } else {
            panic!(
                "Attempt to borrow object using out-of-bounds handle {:?}!, Record count is {}",
                handle, records_len
            );
        }
    }

    pub fn at(&self, n: u32) -> Option<&T> {
        self.records_get(n).and_then(|rec| rec.slot.as_ref())
    }

    pub fn at_mut(&mut self, n: u32) -> Option<&mut T> {
        self.records_get_mut(n).and_then(|rec| rec.slot.as_mut())
    }
}

/// Negative values - amount of mutable borrows, positive - amount of immutable borrows
#[derive(Default, Debug)]
struct RefCounter(pub UnsafeCell<isize>);

unsafe impl Sync for RefCounter {}
unsafe impl Send for RefCounter {}

impl RefCounter {
    unsafe fn get(&self) -> isize {
        unsafe { *self.0.get() }
    }

    unsafe fn increment(&self) {
        unsafe { *self.0.get() += 1 }
    }

    unsafe fn decrement(&self) {
        unsafe { *self.0.get() -= 1 }
    }
}

/// Pool Record is the core container of the pool.
/// It is a warpper for slot.
#[derive(Debug)]
struct PoolRecord<T, S = Option<T>>
where
    T: Sized,
    S: Slot<Element = T>,
{
    ref_counter: RefCounter,
    // The handle is valid only if record it points to is of the same generation at the pool record.
    // Zero is for unknon generation used for None handles
    generation: u32,
    // Actual data
    slot: SlotWrapper<S>,
}

impl<T, S> Clone for PoolRecord<T, S>
where
    T: Clone,
    S: Slot<Element = T> + Clone + 'static,
{
    fn clone(&self) -> Self {
        Self {
            // clone a pool should be a brand new pool
            // so we need to clear ref count to avoid counting borrows from the cloned pool
            ref_counter: Default::default(),
            generation: self.generation,
            slot: self.slot.clone(),
        }
    }
}

impl<T, S> PartialEq for PoolRecord<T, S>
where
    T: PartialEq,
    S: Slot<Element = T> + PartialEq,
{
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation && self.slot.get() == other.slot.get()
    }
}

impl<T, S> Default for PoolRecord<T, S>
where
    S: Slot<Element = T> + 'static,
{
    fn default() -> Self {
        Self {
            ref_counter: Default::default(),
            generation: INVALID_GENERATION,
            slot: SlotWrapper::new_empty(),
        }
    }
}

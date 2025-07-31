pub mod handle;
pub mod slot;

use std::{cell::UnsafeCell, marker::PhantomData};

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

    pub fn try_free(&mut self, handle: Handle<T>) -> Option<T> {
        let index = usize::try_from(handle.index).expect("index overflowed usize");

        self.records.get_mut(index).and_then(|record| {
            if record.generation == handle.generation {
                if let Some(slot) = record.slot.take() {
                    self.free_stack.push(handle.index);
                    Some(slot)
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    pub fn borrow(&self, handle: Handle<T>) -> &T {
        // store the records length on stack can let it not depend on borrowing

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
                handle,
                self.records.len()
            );
        }
    }

    pub fn borrow_mut(&mut self, handle: Handle<T>) -> &mut T {
        // store length into stack to avoid borrowing conflict with self.records_get_mut
        // since it will return a reference to its value. The mutable reference lifetime
        // will be the same as the mut self
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

    pub fn try_borrow(&self, handle: Handle<T>) -> Option<&T> {
        self.records_get(handle.index).and_then(|record| {
            if record.generation == handle.generation {
                record.slot.as_ref()
            } else {
                None
            }
        })
    }

    pub fn try_borrow_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        self.records_get_mut(handle.index).and_then(|record| {
            if record.generation == handle.generation {
                record.slot.as_mut()
            } else {
                None
            }
        })
    }

    pub fn at(&self, n: u32) -> Option<&T> {
        self.records_get(n).and_then(|rec| rec.slot.as_ref())
    }

    pub fn at_mut(&mut self, n: u32) -> Option<&mut T> {
        self.records_get_mut(n).and_then(|rec| rec.slot.as_mut())
    }

    pub fn get_capacity(&self) -> u32 {
        u32::try_from(self.records.len()).expect("records.len() overflowed u32")
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.free_stack.clear();
    }

    pub fn is_valid_handle(&self, handle: Handle<T>) -> bool {
        if let Some(record) = self.records_get(handle.index) {
            record.slot.is_some() && record.generation == handle.generation
        } else {
            false
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[derive(Debug, PartialEq, Clone)]
    struct TestData {
        value: i32,
        name: String,
    }

    impl TestData {
        fn new(value: i32, name: &str) -> Self {
            Self {
                value,
                name: name.to_string(),
            }
        }
    }

    #[derive(Debug)]
    struct SelfAwareNode {
        value: i32,
        my_handle: Handle<SelfAwareNode>,
    }

    #[derive(Debug)]
    struct GraphNode {
        data: String,
        self_ref: Handle<GraphNode>,
        neighbors: Vec<Handle<GraphNode>>,
    }

    #[test]
    fn test_pool_creation() {
        let pool: Pool<TestData> = Pool::new();
        assert_eq!(pool.get_capacity(), 0);

        let pool_with_capacity: Pool<TestData> = Pool::with_capacity(10);
        assert_eq!(pool_with_capacity.get_capacity(), 0); // get_capacity() only return the number of elements
    }

    #[test]
    fn test_spawn_and_borrow() {
        let mut pool: Pool<TestData> = Pool::new();

        let test_data = TestData::new(42, "test");
        let handle = pool.spawn(test_data.clone());
        assert!(pool.is_valid_handle(handle));
        assert!(handle.is_some());
        assert_eq!(handle.index(), 0);
        assert_eq!(handle.generation(), 1);

        // borrow data from pool
        let borrowed_handle = pool.borrow(handle);
        assert_eq!(*borrowed_handle, test_data);
        assert_eq!(borrowed_handle.value, 42);
        assert_eq!(borrowed_handle.name, "test");

        assert_eq!(pool.get_capacity(), 1);
    }

    #[test]
    fn test_borrow_mut() {
        let mut pool: Pool<TestData> = Pool::new();
        let handle = pool.spawn(TestData::new(10, "original"));

        {
            let borrowed_mut = pool.borrow_mut(handle);
            borrowed_mut.value = 20;
            borrowed_mut.name = "modified".to_string();
        }

        let borrowed = pool.borrow(handle);

        assert_eq!(borrowed.value, 20);
        assert_eq!(borrowed.name, "modified");
    }

    #[test]
    fn test_try_borrow() {
        let mut pool: Pool<TestData> = Pool::new();
        let handle = pool.spawn(TestData::new(100, "valid"));

        assert!(pool.try_borrow(handle).is_some());
        assert_eq!(pool.try_borrow(handle).unwrap().value, 100);
        let invalid_handle = Handle::new(999, 999);
        assert!(pool.try_borrow(invalid_handle).is_none());

        assert!(pool.try_borrow_mut(handle).is_some());
        pool.try_borrow_mut(handle).unwrap().value = 200;
        assert_eq!(pool.borrow(handle).value, 200);
    }

    #[test]
    fn test_free_and_memory_reuse() {
        let mut pool: Pool<TestData> = Pool::new();

        let data1 = TestData::new(1, "first");
        let handle1 = pool.spawn(data1.clone());
        assert_eq!(handle1.index(), 0);
        assert_eq!(handle1.generation(), 1);

        let freed_data = pool.free(handle1);
        assert_eq!(freed_data, data1);

        assert!(!pool.is_valid_handle(handle1));

        let data2 = TestData::new(2, "second");
        let handle2 = pool.spawn(data2.clone());
        assert_eq!(handle2.index(), 0);
        assert_eq!(handle2.generation(), 2);

        assert!(pool.is_valid_handle(handle2));
        assert!(!pool.is_valid_handle(handle1));

        assert_eq!(*pool.borrow(handle2), data2);
    }

    #[test]
    fn test_try_free() {
        let mut pool: Pool<TestData> = Pool::new();
        let handle = pool.spawn(TestData::new(42, "test"));

        let freed = pool.try_free(handle);
        assert!(freed.is_some());
        assert_eq!(freed.unwrap().value, 42);

        let result = pool.try_free(handle);
        assert!(result.is_none());

        let invalid_handle = Handle::new(999, 999);
        assert!(pool.try_free(invalid_handle).is_none());
    }

    #[test]
    fn test_spawn_with_callback() {
        let mut pool: Pool<TestData> = Pool::new();

        // Callback is the TestData creation
        let handle = pool.spawn_with(|handle| {
            TestData::new(
                handle.index() as i32,
                &format!("handle-{}", handle.generation()),
            )
        });
        let data = pool.borrow(handle);
        assert_eq!(data.value, 0);
        assert_eq!(data.name, "handle-1");

        let handle2 = pool
            .spawn_with(|h| TestData::new(h.index() as i32 * 10, &format!("obj-{}", h.index())));

        let data2 = pool.borrow(handle2);
        assert_eq!(data2.value, 10);
        assert_eq!(data2.name, "obj-1");
    }

    #[test]
    fn test_self_referencing_node() {
        let mut pool: Pool<SelfAwareNode> = Pool::new();

        let handle = pool.spawn_with(|h| SelfAwareNode {
            value: 100,
            my_handle: h,
        });

        let node = pool.borrow(handle);
        assert_eq!(node.my_handle, handle);
        assert_eq!(node.value, 100);
    }

    #[test]
    fn test_graph_node_creation() {
        let mut pool: Pool<GraphNode> = Pool::new();

        let node_handle = pool.spawn_with(|handle| GraphNode {
            data: "Node A".to_string(),
            self_ref: handle,
            neighbors: vec![],
        });

        let node = pool.borrow(node_handle);
        assert_eq!(node.data, "Node A");
        assert_eq!(node.self_ref, node_handle);
        assert!(node.neighbors.is_empty());
    }

    #[test]
    fn test_graph_connections() {
        let mut pool: Pool<GraphNode> = Pool::new();

        let node_a = pool.spawn_with(|handle| GraphNode {
            data: "A".to_string(),
            self_ref: handle,
            neighbors: vec![],
        });

        let node_b = pool.spawn_with(|handle| GraphNode {
            data: "B".to_string(),
            self_ref: handle,
            neighbors: vec![node_a],
        });

        pool.borrow_mut(node_a).neighbors.push(node_b);

        let borrowed_a = pool.borrow(node_a);
        let borrowed_b = pool.borrow(node_b);

        assert_eq!(borrowed_a.neighbors.len(), 1);
        assert_eq!(borrowed_a.neighbors[0], node_b);

        assert_eq!(borrowed_b.neighbors.len(), 1);
        assert_eq!(borrowed_b.neighbors[0], node_a);
    }

    #[test]
    fn test_triangle_graph() {
        let mut pool: Pool<GraphNode> = Pool::new();

        // A-B-C-A
        // current test is a simple version, neighbors ordering depends on adding time
        let node_a = pool.spawn_with(|h| GraphNode {
            data: "A".to_string(),
            self_ref: h,
            neighbors: vec![],
        });

        let node_b = pool.spawn_with(|h| GraphNode {
            data: "B".to_string(),
            self_ref: h,
            neighbors: vec![node_a],
        });

        let node_c = pool.spawn_with(|h| GraphNode {
            data: "C".to_string(),
            self_ref: h,
            neighbors: vec![node_b],
        });

        pool.borrow_mut(node_a).neighbors.extend([node_b, node_c]);
        pool.borrow_mut(node_b).neighbors.push(node_c);
        pool.borrow_mut(node_c).neighbors.push(node_a);
        let a = pool.borrow(node_a);
        assert!(a.neighbors.contains(&node_b));
        assert!(a.neighbors.contains(&node_c));
    }

    #[test]
    #[should_panic(expected = "Attempt to borrow object using out-of-bounds handle")]
    fn test_pool_panic_invalid_handle_borrow() {
        let pool: Pool<TestData> = Pool::new();
        let invalid_handle = Handle::NONE;
        pool.borrow(invalid_handle);
    }

    #[test]
    #[should_panic(expected = "Attempt to borrow object using out-of-bounds handle")]
    fn test_pool_panic_out_of_bounds_handle() {
        let mut pool: Pool<TestData> = Pool::new();
        pool.spawn(TestData::new(1, "test"));

        let out_of_bounds_handle = Handle::new(999, 1);
        pool.borrow(out_of_bounds_handle);
    }

    #[test]
    #[should_panic(expected = "Attempt to use dangling handle")]
    fn test_pool_panic_dangling_handle() {
        let mut pool: Pool<TestData> = Pool::new();
        let handle = pool.spawn(TestData::new(1, "test"));

        pool.free(handle);

        pool.spawn(TestData::new(2, "new"));

        pool.borrow(handle);
    }

    #[test]
    #[should_panic(expected = "Attempt to double free object")]
    fn test_pool_panic_double_free() {
        let mut pool: Pool<TestData> = Pool::new();
        let handle = pool.spawn(TestData::new(1, "test"));

        pool.free(handle);
        pool.free(handle);
    }

    #[test]
    #[should_panic(expected = "Attempt to borrow destroyed object")]
    fn test_pool_panic_borrow_freed_object() {
        let mut pool: Pool<TestData> = Pool::new();
        let handle = pool.spawn(TestData::new(1, "test"));

        pool.free(handle);
        pool.borrow(handle);
    }

    #[test]
    fn test_pool_handle_cross_thread() {
        let pool = Arc::new(Mutex::new(Pool::<TestData>::new()));

        let handle = {
            let mut pool = pool.lock().unwrap();
            pool.spawn(TestData::new(42, "cross-thread"))
        };

        let pool_clone = Arc::clone(&pool);
        let thread_handle = thread::spawn(move || {
            let pool = pool_clone.lock().unwrap();
            let data = pool.borrow(handle);
            assert_eq!(data.value, 42);
            assert_eq!(data.name, "cross-thread");
            data.value
        });

        let reuslt = thread_handle.join().unwrap();
        assert_eq!(reuslt, 42)
    }
}

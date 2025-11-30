//! # Allocator Module
//!
//! Memory pool management system with handle-based resource tracking.
//!
//! This module provides efficient memory allocation and resource management through:
//!
//! - **Pool**: Generic memory pool with automatic free slot tracking
//! - **Handle**: Type-safe, generational handles for resource references
//! - **Slot**: Configurable slot types for different allocation strategies
//!
//! ## Features
//!
//! - **Generational Handles**: Prevents use-after-free bugs through generation counters
//! - **Type Safety**: Compile-time type checking for resource handles
//! - **Memory Efficiency**: Reuses freed slots to minimize memory fragmentation
//! - **Thread Safety**: Support for atomic operations where needed
//!
//! ## Usage
//!
//! ```rust
//! use moonfield_core::allocator::{Pool, Handle};
//!
//! // Create a pool for your resource type
//! struct MyResource {
//!     data: String,
//! }
//!
//! impl MyResource {
//!     fn new() -> Self {
//!         Self { data: "Hello".to_string() }
//!     }
//! }
//!
//! let mut pool: Pool<MyResource> = Pool::new();
//!
//! // Allocate a resource and get a handle
//! let handle = pool.spawn(MyResource::new());
//!
//! // Access the resource using the handle
//! if let Some(resource) = pool.get(handle) {
//!     // Use the resource
//! }
//!
//! // Free the resource
//! pool.free(handle);
//! ```
//!
//! ## Safety
//!
//! - Handles are invalidated when resources are freed
//! - Generation counters prevent use-after-free bugs
//! - All operations are bounds-checked

pub mod handle;
pub mod multiborrow;
pub mod slot;

use std::cell::UnsafeCell;

pub use handle::*;
pub use multiborrow::*;
pub use slot::*;

/// Invalid index constant used to mark uninitialized or invalid handles.
const INVALID_INDEX: u32 = 0;

/// Invalid generation constant used to mark uninitialized or invalid handles.
const INVALID_GENERATION: u32 = 0;

/// A generic memory pool for efficient resource allocation and management.
///
/// The `Pool` provides automatic memory management with handle-based access.
/// It tracks free slots to enable efficient reuse of memory locations.
///
/// # Type Parameters
///
/// * `T` - The type of resources stored in the pool
/// * `S` - The slot type for managing individual pool entries (defaults to `Option<T>`)
///
/// # Examples
///
/// ```rust
/// use moonfield_core::allocator::Pool;
///
/// struct MyResource {
///     data: String,
/// }
///
/// let mut pool: Pool<MyResource> = Pool::new();
/// let handle = pool.spawn(MyResource { data: "Hello".to_string() });
///
/// if let Some(resource) = pool.get(handle) {
///     println!("Resource data: {}", resource.data);
/// }
/// ```
#[derive(Debug)]
pub struct Pool<T, S = Option<T>>
where
    T: Sized,
    S: Slot<Element = T>, {
    /// Internal records storing the actual resources and their metadata.
    records: Vec<PoolRecord<T, S>>,
    /// Stack of free slot indices for efficient reuse.
    free_stack: Vec<u32>,
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

impl<T, S> Pool<T, S>
where
    S: Slot<Element = T> + 'static,
{
    /// Creates a new empty pool.
    ///
    /// # Returns
    ///
    /// A new `Pool` instance with no allocated resources.
    #[inline]
    pub fn new() -> Self {
        Pool { records: Vec::new(), free_stack: Vec::new() }
    }

    /// Creates a new pool with the specified initial capacity.
    ///
    /// # Arguments
    ///
    /// * `capacity` - The initial capacity for the pool
    ///
    /// # Returns
    ///
    /// A new `Pool` instance with the specified capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` overflows `usize`.
    #[inline]
    pub fn with_capacity(capacity: u32) -> Self {
        let capacity =
            usize::try_from(capacity).expect("capacity overflowed usize");
        Self {
            records: Vec::with_capacity(capacity),
            free_stack: Vec::new(),
        }
    }

    /// Returns the number of records in the pool.
    ///
    /// # Returns
    ///
    /// The number of records as a `u32`.
    ///
    /// # Panics
    ///
    /// Panics if the number of records overflows `u32`.
    fn records_len(&self) -> u32 {
        u32::try_from(self.records.len())
            .expect("Number of records overflowed u32")
    }

    /// Gets a reference to a record at the specified index.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the record to retrieve
    ///
    /// # Returns
    ///
    /// `Some(&PoolRecord<T, S>)` if the index is valid, `None` otherwise.
    fn records_get(&self, index: u32) -> Option<&PoolRecord<T, S>> {
        let index = usize::try_from(index).expect("Index overflowed usize");
        self.records.get(index)
    }

    /// Gets a mutable reference to a record at the specified index.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the record to retrieve
    ///
    /// # Returns
    ///
    /// `Some(&mut PoolRecord<T, S>)` if the index is valid, `None` otherwise.
    fn records_get_mut(&mut self, index: u32) -> Option<&mut PoolRecord<T, S>> {
        let index = usize::try_from(index).expect("Index overflowed usize");
        self.records.get_mut(index)
    } 

    /// Allocates a new resource in the pool.
    ///
    /// # Arguments
    ///
    /// * `slot` - The resource to allocate
    ///
    /// # Returns
    ///
    /// A handle to the newly allocated resource.
    pub fn spawn(&mut self, slot: T) -> Handle<T> {
        self.spawn_with(|_| slot)
    }

    /// Allocates a new resource using a closure that receives the handle.
    ///
    /// This method is useful when the resource needs to know its own handle
    /// during construction, such as in node-based data structures that need
    /// to create links to themselves.
    ///
    /// # Arguments
    ///
    /// * `f` - A closure that creates the resource, receiving the handle as a parameter
    ///
    /// # Returns
    ///
    /// A handle to the newly allocated resource.
    ///
    /// # Examples
    ///
    /// ```rust
/// use moonfield_core::allocator::{Pool, Handle};
///
/// struct Node {
///     handle: Handle<Node>,
///     data: String,
/// }
///
/// let mut pool: Pool<Node> = Pool::new();
/// let handle = pool.spawn_with(|handle| Node {
///     handle,
///     data: "Node data".to_string(),
/// });
/// ```
    pub fn spawn_with<F>(&mut self, f: F) -> Handle<T>
    where
        F: FnOnce(Handle<T>) -> T, {
        let index = if let Some(free_index) = self.free_stack.pop() {
            free_index
        } else {
            let index = self.records_len();
            self.records.push(PoolRecord {
                slot: UnsafeCell::new(S::new_empty()),
                generation: 1, // Start with generation 1, not 0
            });
            index
        };

        let record = self.records_get_mut(index).unwrap();
        let generation = record.generation;
        let handle = Handle::new(index, generation);

        unsafe {
            *record.slot.get() = S::new(f(handle));
        }

        handle
    }

    /// Gets a reference to a resource using its handle.
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle of the resource to retrieve
    ///
    /// # Returns
    ///
    /// `Some(&T)` if the handle is valid and the resource exists, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// let handle = pool.spawn("Hello".to_string());
    ///
    /// if let Some(text) = pool.get(handle) {
    ///     println!("Resource: {}", text);
    /// }
    /// ```
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        if handle.is_none() {
            return None;
        }

        let record = self.records_get(handle.index())?;
        if record.generation != handle.generation() {
            return None;
        }

        unsafe { record.slot.get().as_ref()?.as_ref() }
    }

    /// Gets a mutable reference to a resource using its handle.
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle of the resource to retrieve
    ///
    /// # Returns
    ///
    /// `Some(&mut T)` if the handle is valid and the resource exists, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// let handle = pool.spawn("Hello".to_string());
    ///
    /// if let Some(text) = pool.get_mut(handle) {
    ///     text.push_str(" World!");
    /// }
    /// ```
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        if handle.is_none() {
            return None;
        }

        let record = self.records_get_mut(handle.index())?;
        if record.generation != handle.generation() {
            return None;
        }

        unsafe { record.slot.get().as_mut()?.as_mut() }
    }

    /// Frees a resource and invalidates its handle.
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle of the resource to free
    ///
    /// # Returns
    ///
    /// `true` if the resource was successfully freed, `false` if the handle was invalid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// let handle = pool.spawn("Hello".to_string());
    ///
    /// if pool.free(handle) {
    ///     println!("Resource freed successfully");
    /// }
    ///
    /// // The handle is now invalid
    /// assert!(pool.get(handle).is_none());
    /// ```
    pub fn free(&mut self, handle: Handle<T>) -> bool {
        if handle.is_none() {
            return false;
        }

        let record = match self.records_get_mut(handle.index()) {
            Some(record) => record,
            None => return false,
        };

        if record.generation != handle.generation() {
            return false;
        }

        unsafe {
            *record.slot.get() = S::new_empty();
        }
        record.generation += 1;
        self.free_stack.push(handle.index());

        true
    }

    /// Checks if a handle is valid and points to an existing resource.
    ///
    /// # Arguments
    ///
    /// * `handle` - The handle to validate
    ///
    /// # Returns
    ///
    /// `true` if the handle is valid and points to an existing resource, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// let handle = pool.spawn("Hello".to_string());
    ///
    /// assert!(pool.is_valid(handle));
    ///
    /// pool.free(handle);
    /// assert!(!pool.is_valid(handle));
    /// ```
    pub fn is_valid(&self, handle: Handle<T>) -> bool {
        if handle.is_none() {
            return false;
        }

        let record = match self.records_get(handle.index()) {
            Some(record) => record,
            None => return false,
        };

        if record.generation != handle.generation() {
            return false;
        }

        // Check if the slot contains a valid resource
        unsafe { record.slot.get().as_ref().is_some() }
    }

    /// Returns the number of allocated resources in the pool.
    ///
    /// # Returns
    ///
    /// The number of currently allocated resources.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// assert_eq!(pool.len(), 0);
    ///
    /// let handle1 = pool.spawn("First".to_string());
    /// let handle2 = pool.spawn("Second".to_string());
    /// assert_eq!(pool.len(), 2);
    ///
    /// pool.free(handle1);
    /// assert_eq!(pool.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        // Count the number of non-empty slots
        self.records.iter().filter(|record| {
            unsafe { 
                let slot_ref = record.slot.get().as_ref().unwrap();
                slot_ref.is_some()
            }
        }).count()
    }

    /// Checks if the pool is empty (no allocated resources).
    ///
    /// # Returns
    ///
    /// `true` if no resources are allocated, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// assert!(pool.is_empty());
    ///
    /// let handle = pool.spawn("Hello".to_string());
    /// assert!(!pool.is_empty());
    ///
    /// pool.free(handle);
    /// assert!(pool.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the total capacity of the pool.
    ///
    /// # Returns
    ///
    /// The total number of slots available in the pool.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let pool: Pool<String> = Pool::with_capacity(10);
    /// assert_eq!(pool.capacity(), 10);
    /// ```
    pub fn capacity(&self) -> usize {
        self.records.capacity()
    }

    /// Clears all resources from the pool.
    ///
    /// This method frees all allocated resources and resets the pool to its initial state.
    /// All handles become invalid after this operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// let handle1 = pool.spawn("First".to_string());
    /// let handle2 = pool.spawn("Second".to_string());
    ///
    /// pool.clear();
    /// assert!(pool.is_empty());
    /// assert!(!pool.is_valid(handle1));
    /// assert!(!pool.is_valid(handle2));
    /// ```
    pub fn clear(&mut self) {
        self.records.clear();
        self.free_stack.clear();
    }

    /// Returns an iterator over all valid resources in the pool.
    ///
    /// # Returns
    ///
    /// An iterator that yields references to all allocated resources.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// pool.spawn("First".to_string());
    /// pool.spawn("Second".to_string());
    ///
    /// let resources: Vec<&String> = pool.iter().collect();
    /// assert_eq!(resources.len(), 2);
    /// ```
    pub fn iter(&self) -> PoolIter<'_, T, S> {
        PoolIter { pool: self, index: 0 }
    }

    /// Returns an iterator over all valid resources in the pool with mutable references.
    ///
    /// # Returns
    ///
    /// An iterator that yields mutable references to all allocated resources.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use moonfield_core::allocator::Pool;
    ///
    /// let mut pool: Pool<String> = Pool::new();
    /// pool.spawn("Hello".to_string());
    ///
    /// for resource in pool.iter_mut() {
    ///     resource.push_str(" World!");
    /// }
    /// ```
    pub fn iter_mut(&mut self) -> PoolIterMut<'_, T, S> {
        PoolIterMut { pool: self, index: 0 }
    }
}

/// Internal record structure for pool entries.
///
/// This structure holds the actual resource data and metadata for each pool slot.
///
/// # Type Parameters
///
/// * `T` - The type of resources stored in the pool
/// * `S` - The slot type for managing individual pool entries
#[derive(Debug)]
struct PoolRecord<T, S>
where
    T: Sized,
    S: Slot<Element = T>, {
    /// The slot containing the resource data.
    slot: UnsafeCell<S>,
    /// Generation counter for detecting stale handles.
    generation: u32,
}

/// Iterator over immutable references to pool resources.
///
/// This iterator yields references to all valid resources in the pool.
///
/// # Type Parameters
///
/// * `T` - The type of resources stored in the pool
/// * `S` - The slot type for managing individual pool entries
pub struct PoolIter<'a, T, S>
where
    T: Sized,
    S: Slot<Element = T>, {
    /// Reference to the pool being iterated.
    pool: &'a Pool<T, S>,
    /// Current index in the iteration.
    index: usize,
}

impl<'a, T, S> Iterator for PoolIter<'a, T, S>
where
    T: Sized,
    S: Slot<Element = T>,
{
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.pool.records.len() {
            let index = self.index;
            self.index += 1;

            let record = &self.pool.records[index];
            if let Some(resource) =
                unsafe { record.slot.get().as_ref()?.as_ref() }
            {
                return Some(resource);
            }
        }
        None
    }
}

/// Iterator over mutable references to pool resources.
///
/// This iterator yields mutable references to all valid resources in the pool.
///
/// # Type Parameters
///
/// * `T` - The type of resources stored in the pool
/// * `S` - The slot type for managing individual pool entries
pub struct PoolIterMut<'a, T, S>
where
    T: Sized,
    S: Slot<Element = T>, {
    /// Mutable reference to the pool being iterated.
    pool: &'a mut Pool<T, S>,
    /// Current index in the iteration.
    index: usize,
}

impl<'a, T, S> Iterator for PoolIterMut<'a, T, S>
where
    T: Sized,
    S: Slot<Element = T>,
{
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.pool.records.len() {
            let index = self.index;
            self.index += 1;

            let record = &mut self.pool.records[index];
            if let Some(resource) =
                unsafe { record.slot.get().as_mut()?.as_mut() }
            {
                return Some(resource);
            }
        }
        None
    }
}



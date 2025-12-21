//! # Handle Module
//!
//! Type-safe, generational handles for resource management.
//!
//! This module provides handle-based resource management with generation counters
//! to prevent use-after-free bugs. Handles are lightweight references that can be
//! safely copied and passed around, while the actual resources are managed by pools.
//!
//! ## Features
//!
//! - **Type Safety**: Compile-time type checking for resource handles
//! - **Generational Safety**: Generation counters prevent use-after-free bugs
//! - **Lightweight**: Handles are small and can be efficiently copied
//! - **Thread Safe**: Atomic handles for concurrent access
//! - **Type Erasure**: `ErasedHandle` for type-erased handle storage
//!
//! ## Handle Types
//!
//! - **`Handle<T>`**: Type-safe handle with generation counter
//! - **`AtomicHandle`**: Thread-safe atomic handle
//! - **`ErasedHandle`**: Type-erased handle for storage
//!
//! ## Usage
//!
//! ```rust
//! use moonfield_core::allocator::{Handle, AtomicHandle, ErasedHandle};
//!
//! // Create a typed handle
//! let handle: Handle<String> = Handle::new(1, 1);
//!
//! // Create an atomic handle
//! let atomic_handle = AtomicHandle::new(1, 1);
//!
//! // Convert between handle types
//! let erased: ErasedHandle = handle.into();
//! let typed: Handle<String> = erased.into();
//! ```

use std::{
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
    hash::Hash,
    marker::PhantomData,
    sync::atomic::{self, AtomicUsize},
};

use crate::allocator::{INVALID_GENERATION, INVALID_INDEX};

/// A type-safe, generational handle for resource references.
///
/// `Handle<T>` provides a lightweight, copyable reference to resources stored in pools.
/// It uses generation counters to detect when a resource has been freed and reallocated,
/// preventing use-after-free bugs.
///
/// # Type Parameters
///
/// * `T` - The type of resource this handle references
///
/// # Safety
///
/// Handles are invalidated when the referenced resource is freed. Attempting to use
/// an invalid handle will result in `None` being returned from pool access methods.
///
/// # Examples
///
/// ```rust
/// use moonfield_core::allocator::{Handle, Pool};
///
/// let mut pool: Pool<String> = Pool::new();
/// let handle = pool.spawn("Hello".to_string());
///
/// // The handle can be copied and passed around
/// let handle_copy = handle;
///
/// // Access the resource using the handle
/// if let Some(text) = pool.get(handle_copy) {
///     println!("Resource: {}", text);
/// }
/// ```
pub struct Handle<T> {
    /// The index of the resource in the pool.
    pub(crate) index: u32,
    /// The generation counter for detecting stale handles.
    pub(crate) generation: u32,
    /// Phantom data for type safety.
    pub(crate) type_marker: PhantomData<T>,
}

// ============================================================================
// Handle<T> trait implementations
// ============================================================================

impl<T> Default for Handle<T> {
    /// Creates a default handle representing no resource.
    ///
    /// # Returns
    ///
    /// A handle equivalent to `Handle::NONE`.
    #[inline(always)]
    fn default() -> Self {
        Self::NONE
    }
}

impl<T> PartialEq for Handle<T> {
    /// Compares two handles for equality.
    ///
    /// Two handles are equal if they have the same index and generation.
    ///
    /// # Arguments
    ///
    /// * `other` - The handle to compare with
    ///
    /// # Returns
    ///
    /// `true` if the handles are equal, `false` otherwise.
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Clone for Handle<T> {
    /// Creates a copy of this handle.
    ///
    /// Since `Handle<T>` is `Copy`, this simply returns `*self`.
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialOrd for Handle<T> {
    /// Compares two handles for ordering.
    ///
    /// Handles are ordered by their index values.
    ///
    /// # Arguments
    ///
    /// * `other` - The handle to compare with
    ///
    /// # Returns
    ///
    /// `Some(Ordering)` indicating the relative order of the handles.
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Handle<T> {
    /// Compares two handles for ordering.
    ///
    /// Handles are ordered by their index values.
    ///
    /// # Arguments
    ///
    /// * `other` - The handle to compare with
    ///
    /// # Returns
    ///
    /// `Ordering` indicating the relative order of the handles.
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.index.cmp(&other.index)
    }
}

/// Mark `Handle<T>` as `Send` for all types `T`.
///
/// Handles are safe to send between threads as they only contain
/// primitive data and phantom data.
unsafe impl<T> Send for Handle<T> {}

/// Mark `Handle<T>` as `Sync` for all types `T`.
///
/// Handles are safe to share between threads as they only contain
/// primitive data and phantom data.
unsafe impl<T> Sync for Handle<T> {}

impl<T> Display for Handle<T> {
    /// Formats the handle as a string.
    ///
    /// The format is "index:generation".
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    ///
    /// # Returns
    ///
    /// `fmt::Result` indicating success or failure.
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.index, self.generation)
    }
}

impl<T> Debug for Handle<T> {
    /// Formats the handle for debugging.
    ///
    /// The format is "[Idx: index; Gen: generation]".
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    ///
    /// # Returns
    ///
    /// `fmt::Result` indicating success or failure.
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Idx: {}; Gen: {}]", self.index, self.generation)
    }
}

// ============================================================================
// Handle<T> conversion implementations
// ============================================================================

impl<T> From<ErasedHandle> for Handle<T> {
    /// Converts an `ErasedHandle` to a typed `Handle<T>`.
    ///
    /// This conversion preserves the index and generation values.
    ///
    /// # Arguments
    ///
    /// * `erased_handle` - The type-erased handle to convert
    ///
    /// # Returns
    ///
    /// A typed handle with the same index and generation.
    fn from(erased_handle: ErasedHandle) -> Self {
        Handle {
            index: erased_handle.index,
            generation: erased_handle.generation,
            type_marker: PhantomData,
        }
    }
}

impl<T> From<AtomicHandle> for Handle<T> {
    /// Converts an `AtomicHandle` to a typed `Handle<T>`.
    ///
    /// This conversion loads the current values from the atomic handle.
    ///
    /// # Arguments
    ///
    /// * `atomic_handle` - The atomic handle to convert
    ///
    /// # Returns
    ///
    /// A typed handle with the current index and generation values.
    fn from(atomic_handle: AtomicHandle) -> Self {
        Handle {
            index: atomic_handle.index(),
            generation: atomic_handle.generation(),
            type_marker: PhantomData,
        }
    }
}

// ============================================================================
// Handle<T> methods
// ============================================================================

impl<T> Handle<T> {
    /// A handle representing no resource.
    ///
    /// This handle has invalid index and generation values and will
    /// always return `None` when used to access a pool.
    pub const NONE: Handle<T> = Handle {
        index: INVALID_INDEX,
        generation: INVALID_GENERATION,
        type_marker: PhantomData,
    };

    /// Creates a new handle with the specified index and generation.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the resource in the pool
    /// * `generation` - The generation counter for this handle
    ///
    /// # Returns
    ///
    /// A new handle with the specified values.
    #[inline(always)]
    pub fn new(index: u32, generation: u32) -> Self {
        Handle { index, generation, type_marker: PhantomData }
    }

    /// Checks if this handle represents no resource.
    ///
    /// # Returns
    ///
    /// `true` if this handle is `Handle::NONE`, `false` otherwise.
    #[inline(always)]
    pub fn is_none(self) -> bool {
        self.index == 0 && self.generation == INVALID_GENERATION
    }

    /// Checks if this handle represents a valid resource.
    ///
    /// # Returns
    ///
    /// `true` if this handle is not `Handle::NONE`, `false` otherwise.
    #[inline(always)]
    pub fn is_some(self) -> bool {
        !self.is_none()
    }

    /// Returns the index of the resource in the pool.
    ///
    /// # Returns
    ///
    /// The index value of this handle.
    #[inline(always)]
    pub fn index(self) -> u32 {
        self.index
    }

    /// Returns the generation counter of this handle.
    ///
    /// # Returns
    ///
    /// The generation value of this handle.
    #[inline(always)]
    pub fn generation(self) -> u32 {
        self.generation
    }

    /// Changes the type of this handle while preserving index and generation.
    ///
    /// This method is useful for type conversion when the underlying resource
    /// can be safely reinterpreted as a different type.
    ///
    /// # Type Parameters
    ///
    /// * `U` - The new type for the handle
    ///
    /// # Returns
    ///
    /// A new handle with the same index and generation but different type.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the resource at this index can be safely
    /// interpreted as type `U`.
    #[inline(always)]
    pub fn transmute<U>(&self) -> Handle<U> {
        Handle {
            index: self.index,
            generation: self.generation,
            type_marker: PhantomData,
        }
    }

    /// Decodes a handle from a `u128` value.
    ///
    /// This method reconstructs a handle from a previously encoded value.
    ///
    /// # Arguments
    ///
    /// * `num` - The encoded handle value
    ///
    /// # Returns
    ///
    /// A handle with the decoded index and generation values.
    #[inline(always)]
    pub fn decode_from_u128(num: u128) -> Self {
        Self {
            index: num as u32,
            generation: (num >> 32) as u32,
            type_marker: PhantomData,
        }
    }

    /// Encodes this handle to a `u128` value.
    ///
    /// This method allows handles to be stored in a single integer value.
    ///
    /// # Returns
    ///
    /// A `u128` value containing the encoded handle.
    #[inline(always)]
    pub fn encode_to_u128(&self) -> u128 {
        (self.index as u128) | ((self.generation as u128) << 32)
    }
}

// ============================================================================
// AtomicHandle trait implementations
// ============================================================================

/// A thread-safe atomic handle for concurrent access.
///
/// `AtomicHandle` provides atomic operations for handle values, allowing
/// safe concurrent access from multiple threads.
///
/// # Examples
///
/// ```rust
/// use moonfield_core::allocator::{AtomicHandle, Handle};
/// use std::sync::Arc;
/// use std::thread;
///
/// let atomic_handle = Arc::new(AtomicHandle::new(1, 1));
///
/// let handle_clone = Arc::clone(&atomic_handle);
/// thread::spawn(move || {
///     let handle: Handle<String> = Handle::from(handle_clone.as_ref().clone());
///     println!("Thread handle: {:?}", handle);
/// });
/// ```
#[derive(Default)]
pub struct AtomicHandle(AtomicUsize);

impl Clone for AtomicHandle {
    /// Creates a copy of this atomic handle.
    ///
    /// The new handle will have the same current value as this one.
    ///
    /// # Returns
    ///
    /// A new atomic handle with the same current value.
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(AtomicUsize::new(self.0.load(atomic::Ordering::Relaxed)))
    }
}

impl Display for AtomicHandle {
    /// Formats the atomic handle as a string.
    ///
    /// The format is "index:generation".
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    ///
    /// # Returns
    ///
    /// `fmt::Result` indicating success or failure.
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.index(), self.generation())
    }
}

impl Debug for AtomicHandle {
    /// Formats the atomic handle for debugging.
    ///
    /// The format is "[Idx: index; Gen: generation]".
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    ///
    /// # Returns
    ///
    /// `fmt::Result` indicating success or failure.
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Idx: {}; Gen: {}]", self.index(), self.generation())
    }
}

// ============================================================================
// AtomicHandle methods
// ============================================================================

impl AtomicHandle {
    /// Creates a new atomic handle representing no resource.
    ///
    /// # Returns
    ///
    /// A new atomic handle with invalid values.
    pub fn none() -> Self {
        Self(AtomicUsize::new(0))
    }

    /// Creates a new atomic handle with the specified index and generation.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the resource
    /// * `generation` - The generation counter
    ///
    /// # Returns
    ///
    /// A new atomic handle with the specified values.
    #[inline(always)]
    pub fn new(index: u32, generation: u32) -> Self {
        let handle = Self(AtomicUsize::new(0));
        handle.set(index, generation);
        handle
    }

    /// Sets the index and generation values of this atomic handle.
    ///
    /// This operation is atomic and thread-safe.
    ///
    /// # Arguments
    ///
    /// * `index` - The new index value
    /// * `generation` - The new generation value
    #[inline(always)]
    pub fn set(&self, index: u32, generation: u32) {
        // Pack index and generation into a single usize
        // Index goes in the lower half, generation in the upper half
        let index = (index as usize) << (usize::BITS / 2) >> (usize::BITS / 2);
        let generation = (generation as usize) << (usize::BITS / 2);
        self.0.store(index | generation, atomic::Ordering::Relaxed);
    }

    /// Sets this atomic handle from a typed handle.
    ///
    /// # Arguments
    ///
    /// * `handle` - The typed handle to copy values from
    #[inline(always)]
    pub fn set_from_handle<T>(&self, handle: Handle<T>) {
        self.set(handle.index, handle.generation);
    }

    /// Checks if this atomic handle represents a valid resource.
    ///
    /// # Returns
    ///
    /// `true` if this handle has a valid generation, `false` otherwise.
    #[inline(always)]
    pub fn is_some(&self) -> bool {
        self.generation() != INVALID_GENERATION
    }

    /// Checks if this atomic handle represents no resource.
    ///
    /// # Returns
    ///
    /// `true` if this handle has an invalid generation, `false` otherwise.
    #[inline(always)]
    pub fn is_none(&self) -> bool {
        !self.is_some()
    }

    /// Returns the current index value of this atomic handle.
    ///
    /// # Returns
    ///
    /// The current index value.
    #[inline]
    pub fn index(&self) -> u32 {
        let bytes = self.0.load(atomic::Ordering::Relaxed);
        ((bytes << (usize::BITS / 2)) >> (usize::BITS / 2)) as u32
    }

    /// Returns the current generation value of this atomic handle.
    ///
    /// # Returns
    ///
    /// The current generation value.
    #[inline]
    pub fn generation(&self) -> u32 {
        let bytes = self.0.load(atomic::Ordering::Relaxed);
        (bytes >> (usize::BITS / 2)) as u32
    }
}

// ============================================================================
// ErasedHandle trait implementations
// ============================================================================

/// A type-erased handle for storage and serialization.
///
/// `ErasedHandle` removes type information from handles, allowing them to be
/// stored in generic containers or serialized without type information.
///
/// # Examples
///
/// ```rust
/// use moonfield_core::allocator::{Handle, ErasedHandle};
///
/// let typed_handle: Handle<String> = Handle::new(1, 1);
/// let erased: ErasedHandle = typed_handle.into();
///
/// // Store in a generic container
/// let handles: Vec<ErasedHandle> = vec![erased];
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErasedHandle {
    /// The index of the resource in the pool.
    index: u32,
    /// The generation counter for detecting stale handles.
    generation: u32,
}

impl Default for ErasedHandle {
    /// Creates a default erased handle representing no resource.
    ///
    /// # Returns
    ///
    /// An erased handle equivalent to `ErasedHandle::NONE`.
    fn default() -> Self {
        Self::NONE
    }
}

impl<T> From<Handle<T>> for ErasedHandle {
    /// Converts a typed handle to an erased handle.
    ///
    /// This conversion preserves the index and generation values.
    ///
    /// # Arguments
    ///
    /// * `handle` - The typed handle to convert
    ///
    /// # Returns
    ///
    /// An erased handle with the same index and generation.
    fn from(handle: Handle<T>) -> Self {
        Self { index: handle.index, generation: handle.generation }
    }
}

// ============================================================================
// ErasedHandle methods
// ============================================================================

impl ErasedHandle {
    /// An erased handle representing no resource.
    ///
    /// This handle has invalid index and generation values.
    pub const NONE: ErasedHandle =
        ErasedHandle { index: INVALID_INDEX, generation: INVALID_GENERATION };

    /// Creates a new erased handle with the specified index and generation.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the resource in the pool
    /// * `generation` - The generation counter for this handle
    ///
    /// # Returns
    ///
    /// A new erased handle with the specified values.
    #[inline(always)]
    pub fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// Checks if this erased handle represents a valid resource.
    ///
    /// # Returns
    ///
    /// `true` if this handle has a valid generation, `false` otherwise.
    #[inline(always)]
    pub fn is_some(&self) -> bool {
        self.generation != INVALID_GENERATION
    }

    /// Checks if this erased handle represents no resource.
    ///
    /// # Returns
    ///
    /// `true` if this handle has an invalid generation, `false` otherwise.
    #[inline(always)]
    pub fn is_none(&self) -> bool {
        !self.is_some()
    }

    /// Returns the index of the resource in the pool.
    ///
    /// # Returns
    ///
    /// The index value of this handle.
    #[inline(always)]
    pub fn index(self) -> u32 {
        self.index
    }

    /// Returns the generation counter of this handle.
    ///
    /// # Returns
    ///
    /// The generation value of this handle.
    #[inline(always)]
    pub fn generation(self) -> u32 {
        self.generation
    }
}

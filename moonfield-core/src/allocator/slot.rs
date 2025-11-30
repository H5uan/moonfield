//! # Slot Module
//! 
//! Flexible slot abstraction for pool-based resource management.
//! 
//! This module provides a generic slot trait that can be implemented by different
//! types to provide customized resource storage behavior. The default implementation
//! uses `Option<T>` for simple resource storage.
//! 
//! ## Features
//! 
//! - **Generic Slot Trait**: Extensible slot behavior through trait implementation
//! - **Thread-Safe Wrapper**: `LockedSlot` provides internal mutability
//! - **Default Implementation**: `Option<T>` implements the slot trait out of the box
//! 
//! ## Usage
//! 
//! ```rust
//! use moonfield_core::allocator::slot::{Slot, LockedSlot};
//! 
//! // Use the default Option-based slot
//! let slot = Option::<String>::new("Hello".to_string());
//! assert!(slot.is_some());
//! 
//! // Use the thread-safe wrapper
//! let locked_slot: LockedSlot<Option<String>> = LockedSlot::new("Thread Safe".to_string());
//! assert!(locked_slot.is_some());
//! ```

use std::cell::UnsafeCell;

/// A flexible slot abstraction for storing optional values.
/// 
/// This trait provides a generic interface for storing and managing optional values
/// in pool-based resource management systems. It allows for custom slot implementations
/// that can add features like statistics, validation, or other metadata.
/// 
/// # Type Parameters
/// 
/// * `Element` - The type of value stored in the slot
/// 
/// # Examples
/// 
/// ```rust
/// use moonfield_core::allocator::slot::Slot;
/// 
/// // Basic usage with Option
/// let slot = Option::<String>::new("Hello".to_string());
/// assert!(slot.is_some());
/// 
/// if let Some(value) = slot.as_ref() {
///     println!("Value: {}", value);
/// }
/// ```
pub trait Slot: Sized {
    /// The type of element stored in this slot.
    type Element;

    /// Creates a new empty slot.
    /// 
    /// # Returns
    /// 
    /// A new slot instance containing no value.
    fn new_empty() -> Self;

    /// Creates a new slot containing the specified element.
    /// 
    /// # Arguments
    /// 
    /// * `element` - The element to store in the slot
    /// 
    /// # Returns
    /// 
    /// A new slot instance containing the specified element.
    fn new(element: Self::Element) -> Self;

    /// Checks if the slot contains a value.
    /// 
    /// # Returns
    /// 
    /// `true` if the slot contains a value, `false` otherwise.
    fn is_some(&self) -> bool;

    /// Gets an immutable reference to the contained value.
    /// 
    /// # Returns
    /// 
    /// `Some(&Element)` if the slot contains a value, `None` otherwise.
    fn as_ref(&self) -> Option<&Self::Element>;

    /// Gets a mutable reference to the contained value.
    /// 
    /// # Returns
    /// 
    /// `Some(&mut Element)` if the slot contains a value, `None` otherwise.
    fn as_mut(&mut self) -> Option<&mut Self::Element>;

    /// Replaces the contained value with a new one.
    /// 
    /// # Arguments
    /// 
    /// * `element` - The new element to store
    /// 
    /// # Returns
    /// 
    /// The previous element if one existed, `None` otherwise.
    fn replace(&mut self, element: Self::Element) -> Option<Self::Element>;

    /// Removes and returns the contained value.
    /// 
    /// # Returns
    /// 
    /// The contained element if one existed, `None` otherwise.
    fn take(&mut self) -> Option<Self::Element>;
}

/// Default implementation of `Slot` for `Option<T>`.
/// 
/// This implementation provides the simplest possible slot behavior,
/// using Rust's built-in `Option` type for value storage.
/// 
/// # Type Parameters
/// 
/// * `ElementType` - The type of element stored in the option
impl<ElementType> Slot for Option<ElementType> {
    type Element = ElementType;

    #[inline]
    fn new_empty() -> Self {
        Self::None
    }

    #[inline]
    fn new(element: Self::Element) -> Self {
        Self::Some(element)
    }

    #[inline]
    fn is_some(&self) -> bool {
        Option::is_some(self)
    }

    #[inline]
    fn as_ref(&self) -> Option<&Self::Element> {
        Option::as_ref(self)
    }

    #[inline]
    fn as_mut(&mut self) -> Option<&mut Self::Element> {
        Option::as_mut(self)
    }

    #[inline]
    fn replace(&mut self, element: Self::Element) -> Option<Self::Element> {
        Option::replace(self, element)
    }

    #[inline]
    fn take(&mut self) -> Option<Self::Element> {
        Option::take(self)
    }
}

/// A thread-safe wrapper around a slot that provides internal mutability.
/// 
/// This wrapper uses `UnsafeCell` to provide internal mutability, allowing
/// the slot to be modified even when only immutable references are available.
/// Thread safety depends on external synchronization mechanisms.
/// 
/// # Type Parameters
/// 
/// * `S` - The underlying slot type
/// 
/// # Safety
/// 
/// This type is marked as `Send` and `Sync` but requires external synchronization
/// to be thread-safe. The caller must ensure proper locking when accessing from
/// multiple threads.
/// 
/// # Examples
/// 
/// ```rust
/// use moonfield_core::allocator::slot::{Slot, LockedSlot};
/// 
/// let locked_slot: LockedSlot<Option<String>> = LockedSlot::new("Hello".to_string());
/// assert!(locked_slot.is_some());
/// 
/// if let Some(value) = locked_slot.as_ref() {
///     println!("Value: {}", value);
/// }
/// ```
#[derive(Debug)]
pub struct LockedSlot<S>(pub UnsafeCell<S>);

impl<T, S> Clone for LockedSlot<S>
where
    T: Sized,
    S: Slot<Element = T> + Clone,
{
    /// Creates a new `LockedSlot` with the same content as this one.
    /// 
    /// # Returns
    /// 
    /// A new `LockedSlot` instance containing a clone of the current element.
    fn clone(&self) -> Self {
        Self(UnsafeCell::new((self.get()).clone()))
    }
}

impl<T, S> LockedSlot<S>
where
    T: Sized,
    S: Slot<Element = T>,
{
    /// Creates a new `LockedSlot` containing the specified element.
    /// 
    /// # Arguments
    /// 
    /// * `data` - The element to store in the slot
    /// 
    /// # Returns
    /// 
    /// A new `LockedSlot` instance containing the specified element.
    pub fn new(data: T) -> Self {
        Self(UnsafeCell::new(S::new(data)))
    }

    /// Creates a new empty `LockedSlot`.
    /// 
    /// # Returns
    /// 
    /// A new empty `LockedSlot` instance.
    pub fn new_empty() -> Self {
        Self(UnsafeCell::new(S::new_empty()))
    }

    /// Gets an immutable reference to the underlying slot.
    /// 
    /// # Returns
    /// 
    /// An immutable reference to the underlying slot.
    /// 
    /// # Safety
    /// 
    /// This method is safe to call, but the returned reference should not be
    /// used to modify the slot's contents without proper synchronization.
    pub fn get(&self) -> &S {
        unsafe { &*self.0.get() }
    }

    /// Gets a mutable reference to the underlying slot.
    /// 
    /// # Returns
    /// 
    /// A mutable reference to the underlying slot.
    pub fn get_mut(&mut self) -> &mut S {
        self.0.get_mut()
    }

    /// Checks if the slot contains a value.
    /// 
    /// # Returns
    /// 
    /// `true` if the slot contains a value, `false` otherwise.
    pub fn is_some(&self) -> bool {
        self.get().is_some()
    }

    /// Gets an immutable reference to the contained value.
    /// 
    /// # Returns
    /// 
    /// `Some(&T)` if the slot contains a value, `None` otherwise.
    #[inline]
    pub fn as_ref(&self) -> Option<&T> {
        self.get().as_ref()
    }

    /// Gets a mutable reference to the contained value.
    /// 
    /// # Returns
    /// 
    /// `Some(&mut T)` if the slot contains a value, `None` otherwise.
    #[inline]
    pub fn as_mut(&mut self) -> Option<&mut T> {
        self.get_mut().as_mut()
    }

    /// Replaces the contained value with a new one.
    /// 
    /// # Arguments
    /// 
    /// * `element` - The new element to store
    /// 
    /// # Returns
    /// 
    /// The previous element if one existed, `None` otherwise.
    pub fn replace(&mut self, element: T) -> Option<T> {
        self.get_mut().replace(element)
    }

    /// Removes and returns the contained value.
    /// 
    /// # Returns
    /// 
    /// The contained element if one existed, `None` otherwise.
    pub fn take(&mut self) -> Option<T> {
        self.get_mut().take()
    }
}

/// Mark `LockedSlot` as `Sync` when the underlying slot type is `Sync`.
/// 
/// This allows `LockedSlot` to be shared between threads when the underlying
/// slot type is thread-safe.
unsafe impl<S: Sync> Sync for LockedSlot<S> {}

/// Mark `LockedSlot` as `Send` when the underlying slot type is `Send`.
/// 
/// This allows `LockedSlot` to be transferred between threads when the underlying
/// slot type is safe to transfer.
unsafe impl<S: Send> Send for LockedSlot<S> {}

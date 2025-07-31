use std::{
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
    hash::Hash,
    marker::PhantomData,
    sync::atomic::{self, AtomicUsize},
};

use crate::allocator::{INVALID_GENERATION, INVALID_INDEX};

/// Handle is a reference to the content in a pool
/// it only stores index of object and validation
///
/// C++ will not do type check until instantiation
/// so we need the PhantomData to somehow tell compiler somehow we have the data to mark the type of Handle
///
/// when we impl copy, clone will be the same as copy to bitwise copy. Instead of move semantic
pub struct Handle<T> {
    pub index: u32,
    pub generation: u32,
    pub type_marker: PhantomData<T>,
}

impl<T> Default for Handle<T> {
    #[inline(always)]
    fn default() -> Self {
        Self::NONE
    }
}

impl<T> PartialEq for Handle<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Handle<T> {}

impl<T> PartialOrd for Handle<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Handle<T> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.index.cmp(&other.index)
    }
}

unsafe impl<T> Send for Handle<T> {}
unsafe impl<T> Sync for Handle<T> {}

impl<T> Display for Handle<T> {
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.index, self.generation)
    }
}

impl<T> Debug for Handle<T> {
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Idx: {}; Gen: {}]", self.index, self.generation)
    }
}

impl<T> From<ErasedHandle> for Handle<T> {
    fn from(erased_handle: ErasedHandle) -> Self {
        Handle {
            index: erased_handle.index,
            generation: erased_handle.generation,
            type_marker: PhantomData,
        }
    }
}

impl<T> From<AtomicHandle> for Handle<T> {
    fn from(atomic_handle: AtomicHandle) -> Self {
        Handle {
            index: atomic_handle.index(),
            generation: atomic_handle.generation(),
            type_marker: PhantomData,
        }
    }
}

impl<T> Handle<T> {
    pub const NONE: Handle<T> = Handle {
        index: INVALID_INDEX,
        generation: INVALID_GENERATION,
        type_marker: PhantomData,
    };

    #[inline(always)]
    pub fn is_none(self) -> bool {
        self.index == 0 && self.generation == INVALID_GENERATION
    }

    #[inline(always)]
    pub fn is_some(self) -> bool {
        !self.is_none()
    }

    #[inline(always)]
    pub fn index(self) -> u32 {
        self.index
    }

    #[inline(always)]
    pub fn generation(self) -> u32 {
        self.generation
    }

    #[inline(always)]
    pub fn new(index: u32, generation: u32) -> Self {
        Handle {
            index,
            generation,
            type_marker: PhantomData,
        }
    }

    /// change the type and keep the index and generation
    #[inline(always)]
    pub fn transmute<U>(&self) -> Handle<U> {
        Handle {
            index: self.index,
            generation: self.generation,
            type_marker: PhantomData,
        }
    }

    #[inline(always)]
    pub fn decode_from_u128(num: u128) -> Self {
        Self {
            index: num as u32,
            generation: (num >> 32) as u32,
            type_marker: PhantomData,
        }
    }

    #[inline(always)]
    pub fn encode_to_u128(&self) -> u128 {
        (self.index as u128) | ((self.generation as u128) << 32)
    }
}

#[derive(Default)]
pub struct AtomicHandle(AtomicUsize);

impl Clone for AtomicHandle {
    #[inline(always)]
    fn clone(&self) -> Self {
        Self(AtomicUsize::new(self.0.load(atomic::Ordering::Relaxed)))
    }
}

impl Display for AtomicHandle {
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.index(), self.generation())
    }
}

impl Debug for AtomicHandle {
    #[inline(always)]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[Idx: {}; Gen: {}]", self.index(), self.generation())
    }
}

impl AtomicHandle {
    pub fn none() -> Self {
        Self(AtomicUsize::new(0))
    }
    
    #[inline(always)]
    pub fn new(index: u32, generation: u32) -> Self {
        let handle = Self(AtomicUsize::new(0));
        handle.set(index, generation);
        handle
    }

    #[inline(always)]
    pub fn set(&self, index: u32, generation: u32) {
        // first shift left and then shift right to clear
        let index = (index as usize) << (usize::BITS / 2) >> (usize::BITS / 2);
        let generation = (generation as usize) << (usize::BITS / 2);
        self.0.store(index | generation, atomic::Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn set_from_handle<T>(&self, handle: Handle<T>) {
        self.set(handle.index, handle.generation);
    }

    #[inline(always)]
    pub fn is_some(&self) -> bool {
        self.generation() != INVALID_GENERATION
    }

    #[inline(always)]
    pub fn is_none(&self) -> bool {
        !self.is_some()
    }

    #[inline]
    pub fn index(&self) -> u32 {
        let bytes = self.0.load(atomic::Ordering::Relaxed);
        ((bytes << (usize::BITS / 2)) >> (usize::BITS / 2)) as u32
    }

    #[inline]
    pub fn generation(&self) -> u32 {
        let bytes = self.0.load(atomic::Ordering::Relaxed);
        (bytes >> (usize::BITS / 2)) as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErasedHandle {
    index: u32,
    generation: u32,
}

impl Default for ErasedHandle {
    fn default() -> Self {
        Self::NONE
    }
}

impl<T> From<Handle<T>> for ErasedHandle {
    fn from(handle: Handle<T>) -> Self {
        Self {
            index: handle.index,
            generation: handle.generation,
        }
    }
}

impl ErasedHandle {
    pub const NONE: ErasedHandle = ErasedHandle {
        index: INVALID_INDEX,
        generation: INVALID_GENERATION,
    };

    #[inline(always)]
    pub fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    #[inline(always)]
    pub fn is_some(&self) -> bool {
        self.generation != INVALID_GENERATION
    }

    #[inline(always)]
    pub fn is_none(&self) -> bool {
        !self.is_some()
    }

    #[inline(always)]
    pub fn index(self) -> u32 {
        self.index
    }

    #[inline(always)]
    pub fn generation(self) -> u32 {
        self.generation
    }
}

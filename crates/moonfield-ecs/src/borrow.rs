use core::sync::atomic::{AtomicUsize, Ordering};

/// Bit layout of the internal counter: `[1 bit write lock | 63 bits shared count]`.
const WRITE_LOCK_BIT: usize = !(usize::MAX >> 1);

/// Bitmask isolating the shared-borrow count from the write-lock bit.
const SHARED_COUNT_MASK: usize = usize::MAX >> 1;

/// A lightweight atomic borrow counter modelled on [`RefCell`]'s borrow semantics.
///
/// Tracks the number of outstanding shared borrows and a single exclusive borrow
/// atomically, enabling lock-free read-write arbitration without a full mutex.
///
/// # Bit layout
///
/// ```text
/// | 63 (write) | 62 … 0 (shared count) |
/// ```
///
/// - **MSB = 1**: a unique (mutable) borrow is active.
/// - **Bits 62–0**: number of active shared (immutable) borrows.
///
/// # Panics
///
/// Debug assertions will panic on logic errors (e.g., unbalanced releases).
pub struct AtomicBorrow(AtomicUsize);

impl AtomicBorrow {
    /// Creates an unlocked borrow counter.
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    /// Attempts to acquire a shared (immutable) borrow.
    ///
    /// Returns `true` on success, `false` if a unique borrow is currently active.
    ///
    /// # Panics
    ///
    /// Panics if the shared-borrow counter would overflow (more than `usize::MAX >> 1`
    /// simultaneous shared borrows).
    pub fn try_borrow(&self) -> bool {
        let prev = self.0.fetch_add(1, Ordering::Acquire);

        if prev & SHARED_COUNT_MASK == SHARED_COUNT_MASK {
            core::panic!("shared borrow counter overflowed");
        }

        if prev & WRITE_LOCK_BIT != 0 {
            // Write lock held — roll back the increment.
            self.0.fetch_sub(1, Ordering::Release);
            false
        } else {
            true
        }
    }

    /// Attempts to acquire a unique (mutable) borrow.
    ///
    /// Returns `true` on success, `false` if any borrow (shared or unique) is active.
    pub fn try_borrow_mut(&self) -> bool {
        self.0
            .compare_exchange(0, WRITE_LOCK_BIT, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    /// Releases a previously acquired shared borrow.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if no shared borrow is active, or if a unique borrow
    /// is mistakenly released as shared.
    pub fn release_shared(&self) {
        let value = self.0.fetch_sub(1, Ordering::Release);
        debug_assert!(value != 0, "unbalanced release_shared");
        debug_assert!(
            value & WRITE_LOCK_BIT == 0,
            "shared release while unique borrow is active"
        );
    }

    /// Releases a previously acquired unique borrow.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if no unique borrow is active.
    pub fn release_unique(&self) {
        let value = self.0.fetch_and(!WRITE_LOCK_BIT, Ordering::Release);
        debug_assert_ne!(
            value & WRITE_LOCK_BIT,
            0,
            "unique release while no unique borrow is active"
        );
    }
}

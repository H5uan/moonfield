use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};

const UNIQUE_BIT: usize = !(usize::MAX >> 1);

const COUNTER_MASK: usize = usize::MAX >> 1;

/// A thread-safe borrow checker that allows for multiple shared borrows or a single unique borrow.
///
/// This is a core component for ensuring memory safety within the ECS (Entity Component System)
/// at runtime. It uses an `AtomicUsize` to track the borrowing state.
///
/// The state is encoded as follows:
/// - The most significant bit (MSB) is the "unique" bit. If set, a unique borrow is active.
/// - The remaining bits form a counter for the number of shared borrows.
///
/// # Panics
///
/// This structure will abort the process if the shared borrow counter overflows, as this
/// indicates a catastrophic failure in borrow management.

#[repr(align(64))]
pub struct SharedRuntimeBorrow(AtomicUsize);

impl SharedRuntimeBorrow {
    /// Creates a new `SharedRuntimeBorrow` with no active borrows.
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    /// Attempts to acquire a shared (read-only) borrow.
    ///
    /// This increments the shared borrow counter. It will fail if a unique (write) borrow is
    /// currently active.
    ///
    /// # Returns
    ///
    /// - `true` if the shared borrow was successfully acquired.
    /// - `false` if a unique borrow is active, preventing a shared borrow.
    ///
    /// # Panics
    ///
    /// Aborts the process if the shared borrow counter overflows. This is a safeguard against
    /// runaway borrow acquisition, which would indicate a severe bug.
    pub fn borrow(&self) -> bool {
        let prev_value = self.0.fetch_add(1, Ordering::Acquire);

        // Abort on counter overflow, as this is a catastrophic and unrecoverable error.
        if prev_value & COUNTER_MASK == COUNTER_MASK {
            process::abort();
        }

        // If the unique bit was already set, the borrow fails. Roll back the counter.
        if prev_value & UNIQUE_BIT != 0 {
            self.0.fetch_sub(1, Ordering::Release);
            return false;
        }

        true
    }

    /// Attempts to acquire a unique (mutable) borrow.
    ///
    /// This can only succeed if there are no other active borrows (neither shared nor unique).
    /// It works by atomically swapping the state from 0 to `UNIQUE_BIT`.
    ///
    /// # Returns
    ///
    /// - `true` if the unique borrow was successfully acquired.
    /// - `false` if any other borrow (shared or unique) was already active.
    pub fn borrow_mut(&self) -> bool {
        self.0
            .compare_exchange(
                0,
                UNIQUE_BIT,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    /// Releases a previously acquired shared borrow.
    ///
    /// This decrements the shared borrow counter.
    ///
    /// # Panics
    ///
    /// This method will panic in debug builds if it detects an unbalanced release, such as:
    /// - Releasing a shared borrow when the counter is zero.
    /// - Releasing a shared borrow when a unique borrow is active.
    pub fn release(&self) {
        let prev_value = self.0.fetch_sub(1, Ordering::Release);
        debug_assert_ne!(prev_value & COUNTER_MASK, 0, "unbalanced release");
        debug_assert_eq!(
            prev_value & UNIQUE_BIT,
            0,
            "shared release of unique borrow"
        );
    }

    /// Releases a previously acquired unique borrow.
    ///
    /// This clears the unique bit, allowing other borrows to be acquired.
    ///
    /// # Panics
    ///
    /// This method will panic in debug builds if it attempts to release a unique borrow
    /// that was not active.
    pub fn release_mut(&self) {
        let prev_value = self.0.fetch_and(!UNIQUE_BIT, Ordering::Release);
        debug_assert_ne!(
            prev_value & UNIQUE_BIT,
            0,
            "releasing a unique borrow that was not acquired"
        );
    }
}

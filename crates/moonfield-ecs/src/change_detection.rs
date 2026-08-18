//! Change detection: wrapping change ticks and tick-aware reference wrappers.
//!
//! Every component records the tick at which it was added and the tick at
//! which it was last mutably dereferenced. The world advances a global change
//! tick once per schedule run, and each system remembers the window
//! `(last_run, this_run)` it last executed in, so filters can answer "was this
//! component added/changed since I last ran?" without per-entity bookkeeping.

/// The world tick is clamped and rescanned every `CHECK_TICK_THRESHOLD`
/// increments, so relative ages never overflow.
pub const CHECK_TICK_THRESHOLD: u32 = 518_400_000;

/// The maximum relative age a change tick can have. Comparisons clamp to this
/// age, so wraparound can never make a genuinely recent change compare as
/// "unchanged", and two ancient ticks compare deterministically.
pub const MAX_CHANGE_AGE: u32 = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1);

/// A point on the world's change-detection clock.
///
/// Ticks wrap around at `u32::MAX`; comparisons are always made relative to a
/// system's run window via [`Tick::is_newer_than`], which is wraparound-safe.
#[derive(Copy, Clone, Default, Debug, Eq, Hash, PartialEq)]
pub struct Tick {
    tick: u32,
}

impl Tick {
    /// The maximum relative age for a change tick.
    pub const MAX: Self = Self::new(MAX_CHANGE_AGE);

    /// Creates a new [`Tick`] wrapping the given value.
    #[inline]
    pub const fn new(tick: u32) -> Self {
        Self { tick }
    }

    /// Gets the value of this change tick.
    #[inline]
    pub const fn get(self) -> u32 {
        self.tick
    }

    /// Returns `true` if this `Tick` occurred since the system's `last_run`.
    ///
    /// `this_run` is the current tick, used as a reference to deal with
    /// wraparound. Both ages clamp at [`MAX_CHANGE_AGE`], so a genuinely
    /// recent change always compares as newer, while two ancient ticks
    /// compare deterministically instead of wraparound manufacturing a
    /// spurious change report.
    #[inline]
    pub fn is_newer_than(self, last_run: Tick, this_run: Tick) -> bool {
        // Wraparound-safe because `this_run` is always newer than both
        // `last_run` and `self`, and ticks are periodically clamped so the
        // differences never overflow.
        let ticks_since_insert = this_run.relative_to(self).tick.min(MAX_CHANGE_AGE);
        let ticks_since_system = this_run.relative_to(last_run).tick.min(MAX_CHANGE_AGE);

        ticks_since_system > ticks_since_insert
    }

    /// Returns the wrapped distance from `other` to `self` as a tick.
    #[inline]
    pub(crate) fn relative_to(self, other: Self) -> Self {
        let tick = self.tick.wrapping_sub(other.tick);
        Self { tick }
    }

    /// Clamps this tick if it is older than [`Tick::MAX`] relative to
    /// `present`. Returns `true` if clamping was performed.
    #[inline]
    pub fn check_tick(&mut self, present: Tick) -> bool {
        let age = present.relative_to(*self);
        if age.get() > Self::MAX.get() {
            *self = present.relative_to(Self::MAX);
            true
        } else {
            false
        }
    }
}

/// Records when a component was added and when it was last changed.
#[derive(Copy, Clone, Debug)]
pub struct ComponentTicks {
    /// Tick recording when the component was added to its current owner.
    pub added: Tick,
    /// Tick recording the last mutable access (or the add tick).
    pub changed: Tick,
}

/// Tick-aware shared reference to a component.
///
/// Behaves like `&T` via [`Deref`], and additionally reports whether the
/// component was added or changed within the `(last_run, this_run)` window it
/// was created with.
pub struct Ref<'w, T: crate::Component> {
    value: &'w T,
    ticks: ComponentTicks,
    last_run: Tick,
    this_run: Tick,
}

impl<'w, T: crate::Component> Ref<'w, T> {
    pub(crate) fn new(value: &'w T, ticks: ComponentTicks, last_run: Tick, this_run: Tick) -> Self {
        Self {
            value,
            ticks,
            last_run,
            this_run,
        }
    }

    /// Whether the component was added since `last_run`.
    pub fn is_added(&self) -> bool {
        self.ticks.added.is_newer_than(self.last_run, self.this_run)
    }

    /// Whether the component was mutably accessed since `last_run`.
    pub fn is_changed(&self) -> bool {
        self.ticks
            .changed
            .is_newer_than(self.last_run, self.this_run)
    }
}

impl<T: crate::Component> std::ops::Deref for Ref<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

/// Tick-aware unique reference to a component.
///
/// Behaves like `&mut T`; the first mutable dereference records the current
/// tick as the component's changed tick.
pub struct Mut<'w, T: crate::Component> {
    value: *mut T,
    ticks: *mut ComponentTicks,
    last_run: Tick,
    this_run: Tick,
    _marker: std::marker::PhantomData<&'w mut T>,
}

impl<'w, T: crate::Component> Mut<'w, T> {
    /// # Safety
    ///
    /// `value` and `ticks` must point at a live component row and its parallel
    /// tick row, uniquely borrowed for `'w`.
    pub(crate) unsafe fn new(
        value: *mut T,
        ticks: *mut ComponentTicks,
        last_run: Tick,
        this_run: Tick,
    ) -> Self {
        Self {
            value,
            ticks,
            last_run,
            this_run,
            _marker: std::marker::PhantomData,
        }
    }

    /// Whether the component was added since `last_run`.
    pub fn is_added(&self) -> bool {
        // SAFETY: the tick row is valid for the lifetime of this wrapper.
        let ticks = unsafe { &*self.ticks };
        ticks.added.is_newer_than(self.last_run, self.this_run)
    }

    /// Whether the component was mutably accessed since `last_run`.
    pub fn is_changed(&self) -> bool {
        // SAFETY: the tick row is valid for the lifetime of this wrapper.
        let ticks = unsafe { &*self.ticks };
        ticks.changed.is_newer_than(self.last_run, self.this_run)
    }

    /// Records the current tick as the component's changed tick.
    fn mark_changed(&mut self) {
        // SAFETY: unique access is guaranteed by the borrow rules that
        // produced this wrapper.
        unsafe {
            (*self.ticks).changed = self.this_run;
        }
    }
}

impl<T: crate::Component> std::ops::Deref for Mut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: the value pointer is valid and shared access is sound.
        unsafe { &*self.value }
    }
}

impl<T: crate::Component> std::ops::DerefMut for Mut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.mark_changed();
        // SAFETY: the value pointer is valid and uniquely borrowed.
        unsafe { &mut *self.value }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_is_newer_within_window() {
        let added = Tick::new(5);
        // System window: last ran at 3, running now at 8.
        assert!(added.is_newer_than(Tick::new(3), Tick::new(8)));
        // Component older than the system's last run: not newer.
        assert!(!added.is_newer_than(Tick::new(6), Tick::new(8)));
        // Component added exactly at last_run boundary: already seen.
        assert!(!added.is_newer_than(Tick::new(5), Tick::new(8)));
    }

    #[test]
    fn tick_comparison_survives_wraparound() {
        // World tick wrapped past u32::MAX: this_run is small, older ticks are large.
        let this_run = Tick::new(3);
        let last_run = Tick::new(u32::MAX - 1);
        let freshly_added = Tick::new(2); // after wrap
        let before_wrap = Tick::new(u32::MAX - 2);

        assert!(freshly_added.is_newer_than(last_run, this_run));
        assert!(!before_wrap.is_newer_than(last_run, this_run));
    }

    #[test]
    fn tick_comparison_clamps_at_max_age() {
        let this_run = Tick::new(MAX_CHANGE_AGE + 100);
        // A component changed recently still compares as newer even when the
        // system's last run is far in the past: recent changes are never
        // silently skipped.
        let recent = Tick::new(MAX_CHANGE_AGE + 90);
        let ancient_last_run = Tick::new(20);
        assert!(recent.is_newer_than(ancient_last_run, this_run));

        // Two ancient ticks compare deterministically as not-newer, so
        // wraparound can never manufacture a spurious change report.
        let ancient_component = Tick::new(10);
        assert!(!ancient_component.is_newer_than(ancient_last_run, this_run));
    }

    #[test]
    fn check_tick_clamps_ticks_older_than_max_age() {
        let present = Tick::new(MAX_CHANGE_AGE.wrapping_add(50));
        let mut stale = Tick::new(10);
        assert!(stale.check_tick(present));
        // After clamping, the tick reads as exactly MAX_CHANGE_AGE old.
        assert_eq!(present.relative_to(stale).get(), MAX_CHANGE_AGE);

        let mut fresh = Tick::new(MAX_CHANGE_AGE + 40);
        assert!(!fresh.check_tick(present));
        assert_eq!(fresh.get(), MAX_CHANGE_AGE + 40);
    }
}

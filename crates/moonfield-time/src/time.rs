//! The generic clock, ported from `bevy_time::time` (architecture-level; no
//! reflection, single-threaded).
//!
//! [`Time<T>`] is parameterized by a *context* type (`()`, [`Real`](crate::Real),
//! [`Virtual`](crate::Virtual), …) and tracks how much it advanced since the
//! previous update ([`delta`](Time::delta)) and since creation
//! ([`elapsed`](Time::elapsed)).

use std::time::Duration;

/// A generic clock resource that tracks how much it has advanced since its
/// previous update and since its creation.
///
/// The instances used by the engine (inserted by `TimePlugin` in
/// moonfield-app and advanced once per frame by the windowing backend via
/// [`update_time`](crate::update_time)):
///
/// - [`Time<Real>`](crate::Real) tracks real wall-clock time elapsed.
/// - [`Time<Virtual>`](crate::Virtual) tracks virtual game time that may be
///   paused or scaled.
/// - `Time` is the generic "current" clock systems should read by default; it
///   mirrors [`Time<Virtual>`](crate::Virtual) (there is no fixed-update
///   schedule yet, so unlike Bevy it is never swapped for a fixed clock).
///
/// New custom clocks can be created with [`new_with`](Time::new_with) over a
/// user context type and advanced manually with [`advance_by`](Time::advance_by)
/// / [`advance_to`](Time::advance_to).
#[derive(Debug, Copy, Clone)]
pub struct Time<T: Default = ()> {
    context: T,
    wrap_period: Duration,
    delta: Duration,
    delta_secs: f32,
    delta_secs_f64: f64,
    elapsed: Duration,
    elapsed_secs: f32,
    elapsed_secs_f64: f64,
    elapsed_wrapped: Duration,
    elapsed_secs_wrapped: f32,
    elapsed_secs_wrapped_f64: f64,
}

impl<T: Default> Time<T> {
    const DEFAULT_WRAP_PERIOD: Duration = Duration::from_secs(3600); // 1 hour

    /// Create a new clock from `context`, with delta and elapsed starting at
    /// zero.
    pub fn new_with(context: T) -> Self {
        Self {
            context,
            ..Default::default()
        }
    }

    /// Advance this clock by `delta`. [`Duration::ZERO`] is allowed and sets
    /// [`delta`](Self::delta) to zero; the clock never moves backwards.
    pub fn advance_by(&mut self, delta: Duration) {
        self.delta = delta;
        self.delta_secs = self.delta.as_secs_f32();
        self.delta_secs_f64 = self.delta.as_secs_f64();
        self.elapsed += delta;
        self.elapsed_secs = self.elapsed.as_secs_f32();
        self.elapsed_secs_f64 = self.elapsed.as_secs_f64();
        self.elapsed_wrapped = duration_rem(self.elapsed, self.wrap_period);
        self.elapsed_secs_wrapped = self.elapsed_wrapped.as_secs_f32();
        self.elapsed_secs_wrapped_f64 = self.elapsed_wrapped.as_secs_f64();
    }

    /// Advance this clock to a specific `elapsed` time.
    ///
    /// # Panics
    ///
    /// Panics if `elapsed` is less than [`Self::elapsed`].
    pub fn advance_to(&mut self, elapsed: Duration) {
        assert!(
            elapsed >= self.elapsed,
            "tried to move time backwards to an earlier elapsed moment"
        );
        self.advance_by(elapsed - self.elapsed);
    }

    /// The modulus used to calculate [`elapsed_wrapped`](Self::elapsed_wrapped).
    /// Defaults to one hour.
    #[inline]
    pub fn wrap_period(&self) -> Duration {
        self.wrap_period
    }

    /// Set the modulus used to calculate [`elapsed_wrapped`](Self::elapsed_wrapped).
    /// Takes effect on the next advance.
    ///
    /// # Panics
    ///
    /// Panics if `wrap_period` is zero.
    #[inline]
    pub fn set_wrap_period(&mut self, wrap_period: Duration) {
        assert!(!wrap_period.is_zero(), "division by zero");
        self.wrap_period = wrap_period;
    }

    /// How much time advanced since the last update, as a [`Duration`].
    #[inline]
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// How much time advanced since the last update, as [`f32`] seconds.
    #[inline]
    pub fn delta_secs(&self) -> f32 {
        self.delta_secs
    }

    /// How much time advanced since the last update, as [`f64`] seconds.
    #[inline]
    pub fn delta_secs_f64(&self) -> f64 {
        self.delta_secs_f64
    }

    /// How much time advanced since startup, as a [`Duration`].
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// How much time advanced since startup, as [`f32`] seconds.
    ///
    /// Monotonically increasing; precision degrades over time. For an `f32`
    /// without that loss (e.g. shader time uniforms) use
    /// [`elapsed_secs_wrapped`](Self::elapsed_secs_wrapped).
    #[inline]
    pub fn elapsed_secs(&self) -> f32 {
        self.elapsed_secs
    }

    /// How much time advanced since startup, as [`f64`] seconds.
    #[inline]
    pub fn elapsed_secs_f64(&self) -> f64 {
        self.elapsed_secs_f64
    }

    /// [`elapsed`](Self::elapsed) modulo the [`wrap_period`](Self::wrap_period),
    /// as a [`Duration`].
    #[inline]
    pub fn elapsed_wrapped(&self) -> Duration {
        self.elapsed_wrapped
    }

    /// [`elapsed`](Self::elapsed) modulo the [`wrap_period`](Self::wrap_period),
    /// as [`f32`] seconds.
    #[inline]
    pub fn elapsed_secs_wrapped(&self) -> f32 {
        self.elapsed_secs_wrapped
    }

    /// [`elapsed`](Self::elapsed) modulo the [`wrap_period`](Self::wrap_period),
    /// as [`f64`] seconds.
    #[inline]
    pub fn elapsed_secs_wrapped_f64(&self) -> f64 {
        self.elapsed_secs_wrapped_f64
    }

    /// The context of this specific clock.
    #[inline]
    pub fn context(&self) -> &T {
        &self.context
    }

    /// A mutable reference to the context of this specific clock.
    #[inline]
    pub fn context_mut(&mut self) -> &mut T {
        &mut self.context
    }

    /// A copy of this clock as a fully generic clock without context. Used to
    /// refresh the `Time` resource from [`Time<Virtual>`](crate::Virtual)
    /// each frame.
    #[inline]
    pub fn as_generic(&self) -> Time<()> {
        Time {
            context: (),
            wrap_period: self.wrap_period,
            delta: self.delta,
            delta_secs: self.delta_secs,
            delta_secs_f64: self.delta_secs_f64,
            elapsed: self.elapsed,
            elapsed_secs: self.elapsed_secs,
            elapsed_secs_f64: self.elapsed_secs_f64,
            elapsed_wrapped: self.elapsed_wrapped,
            elapsed_secs_wrapped: self.elapsed_secs_wrapped,
            elapsed_secs_wrapped_f64: self.elapsed_secs_wrapped_f64,
        }
    }
}

impl<T: Default> Default for Time<T> {
    fn default() -> Self {
        Self {
            context: Default::default(),
            wrap_period: Self::DEFAULT_WRAP_PERIOD,
            delta: Duration::ZERO,
            delta_secs: 0.0,
            delta_secs_f64: 0.0,
            elapsed: Duration::ZERO,
            elapsed_secs: 0.0,
            elapsed_secs_f64: 0.0,
            elapsed_wrapped: Duration::ZERO,
            elapsed_secs_wrapped: 0.0,
            elapsed_secs_wrapped_f64: 0.0,
        }
    }
}

fn duration_rem(dividend: Duration, divisor: Duration) -> Duration {
    // `Duration` does not have a built-in modulo operation.
    let quotient = (dividend.as_nanos() / divisor.as_nanos()) as u32;
    dividend - (quotient * divisor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_zeroed() {
        let time = Time::<()>::default();
        assert_eq!(time.delta(), Duration::ZERO);
        assert_eq!(time.elapsed(), Duration::ZERO);
        assert_eq!(time.wrap_period(), Duration::from_secs(3600));
    }

    #[test]
    fn test_advance_by_accumulates() {
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_millis(100));
        assert_eq!(time.delta(), Duration::from_millis(100));
        assert_eq!(time.elapsed(), Duration::from_millis(100));
        assert_eq!(time.delta_secs(), 0.1);
        assert_eq!(time.elapsed_secs_f64(), 0.1);

        time.advance_by(Duration::from_millis(50));
        assert_eq!(time.delta(), Duration::from_millis(50));
        assert_eq!(time.elapsed(), Duration::from_millis(150));
    }

    #[test]
    fn test_advance_by_zero_is_allowed() {
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_millis(100));
        time.advance_by(Duration::ZERO);
        assert_eq!(time.delta(), Duration::ZERO);
        assert_eq!(time.elapsed(), Duration::from_millis(100));
    }

    #[test]
    fn test_advance_to_sets_elapsed() {
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs(1));
        time.advance_to(Duration::from_secs(3));
        assert_eq!(time.delta(), Duration::from_secs(2));
        assert_eq!(time.elapsed(), Duration::from_secs(3));
    }

    #[test]
    #[should_panic(expected = "tried to move time backwards")]
    fn test_advance_to_the_past_panics() {
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_secs(2));
        time.advance_to(Duration::from_secs(1));
    }

    #[test]
    fn test_elapsed_wrapped() {
        let mut time = Time::<()>::default();
        time.set_wrap_period(Duration::from_secs(10));
        time.advance_by(Duration::from_secs(25));
        assert_eq!(time.elapsed(), Duration::from_secs(25));
        assert_eq!(time.elapsed_wrapped(), Duration::from_secs(5));
        assert_eq!(time.elapsed_secs_wrapped(), 5.0);
    }

    #[test]
    fn test_as_generic_strips_context() {
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_millis(16));
        let generic = time.as_generic();
        assert_eq!(generic.delta(), time.delta());
        assert_eq!(generic.elapsed(), time.elapsed());
    }
}

//! The real wall-clock time, ported from `bevy_time::real`.
//!
//! [`Time<Real>`] is fed [`Instant`]s once per frame by the windowing backend
//! and is unaffected by pause or time scaling. Tests drive it with synthetic
//! instants via [`update_with_instant`](Time::update_with_instant).

use std::time::{Duration, Instant};

use crate::Time;

/// Real time clock representing elapsed wall-clock time.
///
/// The context of [`Time<Real>`]. `startup` is recorded when the clock is
/// created; the first update only records `first_update`/`last_update` and
/// reports a zero delta (there is no previous update instant to diff
/// against), so time between startup and the first frame is not counted.
#[derive(Debug, Copy, Clone)]
pub struct Real {
    startup: Instant,
    first_update: Option<Instant>,
    last_update: Option<Instant>,
}

impl Default for Real {
    fn default() -> Self {
        Self {
            startup: Instant::now(),
            first_update: None,
            last_update: None,
        }
    }
}

impl Time<Real> {
    /// Constructs a new `Time<Real>` with a specific startup [`Instant`].
    pub fn new(startup: Instant) -> Self {
        Self::new_with(Real {
            startup,
            ..Default::default()
        })
    }

    /// Updates the clock from [`Instant::now`].
    ///
    /// Ordinarily only the windowing backend calls this (once per frame, via
    /// [`update_time`](crate::update_time)); calling it from app code will
    /// disturb timekeeping.
    pub fn update(&mut self) {
        self.update_with_instant(Instant::now());
    }

    /// Updates the clock with a specified [`Instant`]. Provided for tests.
    pub fn update_with_instant(&mut self, instant: Instant) {
        let Some(last_update) = self.context().last_update else {
            let context = self.context_mut();
            context.first_update = Some(instant);
            context.last_update = Some(instant);
            return;
        };
        let delta = instant.saturating_duration_since(last_update);
        self.advance_by(delta);
        self.context_mut().last_update = Some(instant);
    }

    /// Updates the clock as if `duration` had passed since the last update.
    /// Provided for tests.
    pub fn update_with_duration(&mut self, duration: Duration) {
        let last_update = self.context().last_update.unwrap_or(self.context().startup);
        self.update_with_instant(last_update + duration);
    }

    /// The [`Instant`] the clock was created (usually app startup).
    #[inline]
    pub fn startup(&self) -> Instant {
        self.context().startup
    }

    /// The [`Instant`] the clock was first updated, if it has been.
    #[inline]
    pub fn first_update(&self) -> Option<Instant> {
        self.context().first_update
    }

    /// The [`Instant`] the clock was last updated, if it has been.
    #[inline]
    pub fn last_update(&self) -> Option<Instant> {
        self.context().last_update
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_update_reports_zero_delta() {
        let startup = Instant::now();
        let mut time = Time::<Real>::new(startup);

        assert_eq!(time.startup(), startup);
        assert_eq!(time.first_update(), None);
        assert_eq!(time.last_update(), None);

        time.update_with_instant(startup + Duration::from_secs(5));

        // The gap between startup and the first update is not counted.
        assert_eq!(time.first_update(), Some(startup + Duration::from_secs(5)));
        assert_eq!(time.last_update(), Some(startup + Duration::from_secs(5)));
        assert_eq!(time.delta(), Duration::ZERO);
        assert_eq!(time.elapsed(), Duration::ZERO);
    }

    #[test]
    fn test_subsequent_updates_measure_instant_diffs() {
        let startup = Instant::now();
        let mut time = Time::<Real>::new(startup);

        time.update_with_instant(startup + Duration::from_secs(1));
        time.update_with_instant(startup + Duration::from_millis(1016));
        assert_eq!(time.delta(), Duration::from_millis(16));
        assert_eq!(time.elapsed(), Duration::from_millis(16));

        time.update_with_duration(Duration::from_millis(16));
        assert_eq!(time.delta(), Duration::from_millis(16));
        assert_eq!(time.elapsed(), Duration::from_millis(32));
    }

    #[test]
    fn test_out_of_order_instant_saturates_to_zero() {
        let startup = Instant::now();
        let mut time = Time::<Real>::new(startup);
        time.update_with_instant(startup + Duration::from_secs(1));
        // A backwards instant must not panic or move the clock backwards.
        time.update_with_instant(startup);
        assert_eq!(time.delta(), Duration::ZERO);
    }
}

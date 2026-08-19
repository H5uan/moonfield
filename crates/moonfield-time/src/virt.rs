//! The virtual game clock, ported from `bevy_time::virt`.
//!
//! [`Time<Virtual>`] advances from [`Time<Real>`](crate::Real)'s delta each
//! frame, applying pause, relative speed, and a per-update `max_delta` clamp.
//! It is the clock mirrored into the generic [`Time`] resource.

use std::time::Duration;

use crate::{Real, Time};

/// The virtual game clock representing game time.
///
/// The context of [`Time<Virtual>`]. Unlike the real clock it can be paused
/// ([`pause`](Time::pause)/[`unpause`](Time::unpause)/[`toggle`](Time::toggle)),
/// sped up or slowed down ([`set_relative_speed`](Time::set_relative_speed)),
/// and clamps how much it advances in a single update
/// ([`set_max_delta`](Time::set_max_delta)) so a suspended or hitching app
/// does not try to simulate hours of game time in one frame.
#[derive(Debug, Copy, Clone)]
pub struct Virtual {
    max_delta: Duration,
    paused: bool,
    relative_speed: f64,
    effective_speed: f64,
}

impl Time<Virtual> {
    /// The default maximum delta per update: 250 ms.
    const DEFAULT_MAX_DELTA: Duration = Duration::from_millis(250);

    /// Create a new virtual clock with the given maximum delta step.
    ///
    /// # Panics
    ///
    /// Panics if `max_delta` is zero.
    pub fn from_max_delta(max_delta: Duration) -> Self {
        let mut ret = Self::default();
        ret.set_max_delta(max_delta);
        ret
    }

    /// The maximum amount of time a single update can add, as a [`Duration`].
    #[inline]
    pub fn max_delta(&self) -> Duration {
        self.context().max_delta
    }

    /// Set the maximum amount of time a single update can add. Set to
    /// [`Duration::MAX`] to disable the clamp.
    ///
    /// # Panics
    ///
    /// Panics if `max_delta` is zero.
    #[inline]
    pub fn set_max_delta(&mut self, max_delta: Duration) {
        assert_ne!(max_delta, Duration::ZERO, "tried to set max delta to zero");
        self.context_mut().max_delta = max_delta;
    }

    /// The speed the clock advances relative to real time, as [`f32`]
    /// ("time scaling"). `2.0` means twice as fast.
    #[inline]
    pub fn relative_speed(&self) -> f32 {
        self.relative_speed_f64() as f32
    }

    /// The speed the clock advances relative to real time, as [`f64`].
    #[inline]
    pub fn relative_speed_f64(&self) -> f64 {
        self.context().relative_speed
    }

    /// The speed the clock actually advanced relative to real time in this
    /// update, as [`f32`]. `0.0` if paused; below
    /// [`relative_speed`](Self::relative_speed) if the delta was clamped by
    /// [`max_delta`](Self::max_delta).
    #[inline]
    pub fn effective_speed(&self) -> f32 {
        self.context().effective_speed as f32
    }

    /// The speed the clock actually advanced relative to real time in this
    /// update, as [`f64`].
    #[inline]
    pub fn effective_speed_f64(&self) -> f64 {
        self.context().effective_speed
    }

    /// Set the speed the clock advances relative to real time, as [`f32`].
    ///
    /// # Panics
    ///
    /// Panics if `ratio` is negative or not finite.
    #[inline]
    pub fn set_relative_speed(&mut self, ratio: f32) {
        self.set_relative_speed_f64(ratio as f64);
    }

    /// Set the speed the clock advances relative to real time, as [`f64`].
    ///
    /// # Panics
    ///
    /// Panics if `ratio` is negative or not finite.
    #[inline]
    pub fn set_relative_speed_f64(&mut self, ratio: f64) {
        assert!(ratio.is_finite(), "tried to go infinitely fast");
        assert!(ratio >= 0.0, "tried to go back in time");
        self.context_mut().relative_speed = ratio;
    }

    /// Stop the clock if it is running, otherwise resume it.
    #[inline]
    pub fn toggle(&mut self) {
        self.context_mut().paused ^= true;
    }

    /// Stop the clock until resumed. Does not affect the delta of the update
    /// currently being processed.
    #[inline]
    pub fn pause(&mut self) {
        self.context_mut().paused = true;
    }

    /// Resume the clock.
    #[inline]
    pub fn unpause(&mut self) {
        self.context_mut().paused = false;
    }

    /// `true` if the clock is currently paused.
    #[inline]
    pub fn is_paused(&self) -> bool {
        self.context().paused
    }

    /// `true` if the clock was paused at the start of this update.
    #[inline]
    pub fn was_paused(&self) -> bool {
        self.context().effective_speed == 0.0
    }

    /// Advances the clock by `raw_delta * relative_speed`, clamped to
    /// `max_delta`. Called once per frame from
    /// [`update_virtual_time`]; `pub(crate)`-visible tests exercise it
    /// directly.
    pub(crate) fn advance_with_raw_delta(&mut self, raw_delta: Duration) {
        let max_delta = self.context().max_delta;
        let speed = if self.context().paused {
            0.0
        } else {
            self.context().relative_speed
        };
        let scaled = if speed != 1.0 {
            raw_delta.mul_f64(speed)
        } else {
            // Avoid rounding when at normal speed.
            raw_delta
        };
        let (effective_speed, delta) = if scaled > max_delta {
            (max_delta.as_secs_f64() / raw_delta.as_secs_f64(), max_delta)
        } else {
            (speed, scaled)
        };
        self.context_mut().effective_speed = effective_speed;
        self.advance_by(delta);
    }
}

impl Default for Virtual {
    fn default() -> Self {
        Self {
            max_delta: Time::<Virtual>::DEFAULT_MAX_DELTA,
            paused: false,
            relative_speed: 1.0,
            effective_speed: 1.0,
        }
    }
}

/// Advances [`Time<Virtual>`] and the generic [`Time`] from the elapsed
/// [`Time<Real>`] delta. Mirrors Bevy's `time_system` for the virtual clock.
pub fn update_virtual_time(current: &mut Time, virt: &mut Time<Virtual>, real: &Time<Real>) {
    let raw_delta = real.delta();
    virt.advance_with_raw_delta(raw_delta);
    *current = virt.as_generic();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let time = Time::<Virtual>::default();
        assert!(!time.is_paused());
        assert_eq!(time.relative_speed(), 1.0);
        assert_eq!(time.max_delta(), Time::<Virtual>::DEFAULT_MAX_DELTA);
        assert_eq!(time.delta(), Duration::ZERO);
        assert_eq!(time.elapsed(), Duration::ZERO);
    }

    #[test]
    fn test_advance() {
        let mut time = Time::<Virtual>::default();
        for _ in 0..4 {
            time.advance_with_raw_delta(Duration::from_millis(125));
        }
        assert_eq!(time.delta(), Duration::from_millis(125));
        assert_eq!(time.elapsed(), Duration::from_millis(500));
    }

    #[test]
    fn test_relative_speed() {
        let mut time = Time::<Virtual>::default();
        time.set_max_delta(Duration::from_secs(1));

        time.advance_with_raw_delta(Duration::from_millis(250));
        assert_eq!(time.effective_speed(), 1.0);
        assert_eq!(time.delta(), Duration::from_millis(250));

        time.set_relative_speed_f64(2.0);
        // Changing the speed does not affect the update already processed.
        assert_eq!(time.effective_speed(), 1.0);

        time.advance_with_raw_delta(Duration::from_millis(250));
        assert_eq!(time.effective_speed(), 2.0);
        assert_eq!(time.delta(), Duration::from_millis(500));
        assert_eq!(time.elapsed(), Duration::from_millis(750));

        time.set_relative_speed_f64(0.5);
        time.advance_with_raw_delta(Duration::from_millis(250));
        assert_eq!(time.effective_speed(), 0.5);
        assert_eq!(time.delta(), Duration::from_millis(125));
        assert_eq!(time.elapsed(), Duration::from_millis(875));
    }

    #[test]
    fn test_pause() {
        let mut time = Time::<Virtual>::default();
        time.advance_with_raw_delta(Duration::from_millis(250));
        assert!(!time.is_paused());
        assert!(!time.was_paused());

        time.pause();
        time.advance_with_raw_delta(Duration::from_millis(250));
        assert!(time.is_paused());
        assert!(time.was_paused());
        assert_eq!(time.effective_speed(), 0.0);
        assert_eq!(time.delta(), Duration::ZERO);
        assert_eq!(time.elapsed(), Duration::from_millis(250));

        time.unpause();
        time.advance_with_raw_delta(Duration::from_millis(250));
        assert!(!time.is_paused());
        assert!(!time.was_paused());
        assert_eq!(time.delta(), Duration::from_millis(250));
        assert_eq!(time.elapsed(), Duration::from_millis(500));
    }

    #[test]
    fn test_max_delta_clamps_large_steps() {
        let mut time = Time::<Virtual>::default();
        time.set_max_delta(Duration::from_millis(500));

        time.advance_with_raw_delta(Duration::from_millis(750));
        assert_eq!(time.delta(), Duration::from_millis(500));
        assert_eq!(time.elapsed(), Duration::from_millis(500));
        assert!((time.effective_speed() - 500.0 / 750.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_max_delta_clamps_after_relative_speed() {
        let mut time = Time::<Virtual>::default();
        time.set_relative_speed_f64(2000.0);
        time.set_max_delta(Duration::from_secs(1));

        time.advance_with_raw_delta(Duration::from_millis(16));
        assert_eq!(time.delta(), time.max_delta());
        // 62.5 = max_delta / raw_delta = 1000 / 16
        assert_eq!(time.effective_speed(), 62.5);
    }

    #[test]
    fn test_dont_overclamp_at_low_speed() {
        let mut time = Time::<Virtual>::default();
        time.set_relative_speed_f64(0.01);
        time.set_max_delta(Duration::from_millis(10));
        let delta = Duration::from_millis(16);

        time.advance_with_raw_delta(delta);
        assert_eq!(time.delta(), delta / 100);
    }

    #[test]
    fn test_update_virtual_time_mirrors_to_generic() {
        let mut real = Time::<Real>::default();
        real.advance_by(Duration::from_millis(16));
        let mut virt = Time::<Virtual>::default();
        let mut current = Time::default();

        update_virtual_time(&mut current, &mut virt, &real);

        assert_eq!(virt.delta(), Duration::from_millis(16));
        assert_eq!(current.delta(), virt.delta());
        assert_eq!(current.elapsed(), virt.elapsed());
    }
}

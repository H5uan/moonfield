//! The fixed timestep clock, ported from the reference implementation's
//! `bevy_time::fixed` (0.20; architecture-level).
//!
//! [`Time<Fixed>`] advances in fixed-size increments by following
//! [`Time<Virtual>`](crate::Virtual): [`run_fixed_main_schedule`] accumulates
//! the virtual delta into the fixed clock's overstep and runs the fixed
//! schedules once per full timestep. Logic that must be framerate-independent
//! (physics, simulation) runs there.

use std::time::Duration;

use crate::{Time, Virtual};
use moonfield_ecs::World;

/// The fixed timestep game clock following virtual time.
///
/// The context of [`Time<Fixed>`]. `timestep` is the fixed increment;
/// `overstep` is the accumulated virtual time not yet spent on a full step.
/// The default timestep is 64 Hz (15625 µs — a power of two, lossless in
/// `f32`/`f64`, and chosen over 60 Hz to avoid alternating 0/2 steps per
/// frame on 60 Hz displays).
#[derive(Debug, Copy, Clone)]
pub struct Fixed {
    timestep: Duration,
    overstep: Duration,
}

impl Time<Fixed> {
    /// Corresponds to 64 Hz.
    const DEFAULT_TIMESTEP: Duration = Duration::from_micros(15625);

    /// A new fixed clock with the given timestep.
    ///
    /// # Panics
    ///
    /// Panics if `timestep` is zero.
    pub fn from_duration(timestep: Duration) -> Self {
        let mut ret = Self::default();
        ret.set_timestep(timestep);
        ret
    }

    /// A new fixed clock with the given timestep in seconds.
    ///
    /// # Panics
    ///
    /// Panics if `seconds` is zero, negative, or not finite.
    pub fn from_seconds(seconds: f64) -> Self {
        let mut ret = Self::default();
        ret.set_timestep_seconds(seconds);
        ret
    }

    /// A new fixed clock with the given timestep frequency in Hertz.
    ///
    /// # Panics
    ///
    /// Panics if `hz` is zero, negative, or not finite.
    pub fn from_hz(hz: f64) -> Self {
        let mut ret = Self::default();
        ret.set_timestep_hz(hz);
        ret
    }

    /// The amount of virtual time between two fixed steps.
    #[inline]
    pub fn timestep(&self) -> Duration {
        self.context().timestep
    }

    /// Set the amount of virtual time between two fixed steps. Takes effect
    /// on the next step, respecting the current [`overstep`](Self::overstep).
    ///
    /// # Panics
    ///
    /// Panics if `timestep` is zero.
    #[inline]
    pub fn set_timestep(&mut self, timestep: Duration) {
        assert_ne!(
            timestep,
            Duration::ZERO,
            "attempted to set fixed timestep to zero"
        );
        self.context_mut().timestep = timestep;
    }

    /// Set the timestep in seconds.
    ///
    /// # Panics
    ///
    /// Panics if `seconds` is zero, negative, or not finite.
    #[inline]
    pub fn set_timestep_seconds(&mut self, seconds: f64) {
        assert!(
            seconds.is_sign_positive(),
            "seconds less than or equal to zero"
        );
        assert!(seconds.is_finite(), "seconds is infinite");
        self.set_timestep(Duration::from_secs_f64(seconds));
    }

    /// Set the timestep as a frequency in Hertz (`1 / hz` seconds).
    ///
    /// # Panics
    ///
    /// Panics if `hz` is zero, negative, or not finite.
    #[inline]
    pub fn set_timestep_hz(&mut self, hz: f64) {
        assert!(hz.is_sign_positive(), "Hz is less than or equal to zero");
        assert!(hz.is_finite(), "Hz is infinite");
        self.set_timestep_seconds(1.0 / hz);
    }

    /// The time accumulated toward the next step, as a [`Duration`].
    #[inline]
    pub fn overstep(&self) -> Duration {
        self.context().overstep
    }

    /// Add time to the overstep accumulator. Provided for tests and for
    /// [`run_fixed_main_schedule`], which is the ordinary caller.
    #[inline]
    pub fn accumulate_overstep(&mut self, delta: Duration) {
        self.context_mut().overstep += delta;
    }

    /// Discard part of the overstep (saturates to zero).
    #[inline]
    pub fn discard_overstep(&mut self, discard: Duration) {
        let context = self.context_mut();
        context.overstep = context.overstep.saturating_sub(discard);
    }

    /// The accumulated overstep as an [`f32`] fraction of the timestep.
    #[inline]
    pub fn overstep_fraction(&self) -> f32 {
        self.context().overstep.as_secs_f32() / self.context().timestep.as_secs_f32()
    }

    /// The accumulated overstep as an [`f64`] fraction of the timestep.
    #[inline]
    pub fn overstep_fraction_f64(&self) -> f64 {
        self.context().overstep.as_secs_f64() / self.context().timestep.as_secs_f64()
    }

    /// Spend one timestep of overstep, advancing the clock by it. Returns
    /// `false` when less than one timestep remains.
    ///
    /// `pub` (unlike the reference, where it is crate-private) because the
    /// fixed-main driver lives in the app crate here.
    #[inline]
    pub fn expend(&mut self) -> bool {
        let timestep = self.timestep();
        if let Some(new_value) = self.context_mut().overstep.checked_sub(timestep) {
            self.context_mut().overstep = new_value;
            self.advance_by(timestep);
            true
        } else {
            false
        }
    }
}

impl Default for Fixed {
    fn default() -> Self {
        Self {
            timestep: Time::<Fixed>::DEFAULT_TIMESTEP,
            overstep: Duration::ZERO,
        }
    }
}

/// Runs the fixed schedules zero or more times based on [`Time<Virtual>`]'s
/// delta and the [`Time<Fixed>`] overstep accumulator, then restores the
/// generic [`Time`] resource to the virtual clock.
///
/// `run_fixed_main` runs one fixed iteration (the app passes a closure that
/// runs the `FixedFirst` → `FixedPreUpdate` → `FixedUpdate` →
/// `FixedPostUpdate` → `FixedLast` schedules). During each iteration the
/// generic `Time` resource mirrors `Time<Fixed>`, so systems reading
/// `Res<Time>` see the fixed delta.
///
/// No-op when either clock resource is missing (i.e. `TimePlugin` was not
/// added), so apps without time keep working.
pub fn run_fixed_main_schedule(world: &mut World, mut run_fixed_main: impl FnMut(&mut World)) {
    if !world.contains_resource::<Time<Virtual>>() || !world.contains_resource::<Time<Fixed>>() {
        return;
    }
    let delta = world.get_resource::<Time<Virtual>>().unwrap().delta();
    world
        .get_resource_mut::<Time<Fixed>>()
        .unwrap()
        .accumulate_overstep(delta);

    while world.get_resource_mut::<Time<Fixed>>().unwrap().expend() {
        let snapshot = world.get_resource::<Time<Fixed>>().unwrap().as_generic();
        world.insert_resource(snapshot);
        run_fixed_main(world);
    }

    // Restore the generic clock to virtual time for the rest of the frame.
    if world.contains_resource::<Time>() {
        let snapshot = world.get_resource::<Time<Virtual>>().unwrap().as_generic();
        world.insert_resource(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_timestep() {
        let mut time = Time::<Fixed>::default();
        assert_eq!(time.timestep(), Time::<Fixed>::DEFAULT_TIMESTEP);

        time.set_timestep(Duration::from_millis(500));
        assert_eq!(time.timestep(), Duration::from_millis(500));

        time.set_timestep_seconds(0.25);
        assert_eq!(time.timestep(), Duration::from_millis(250));

        time.set_timestep_hz(8.0);
        assert_eq!(time.timestep(), Duration::from_millis(125));
    }

    #[test]
    fn test_from_constructors() {
        assert_eq!(
            Time::<Fixed>::from_hz(4.0).timestep(),
            Duration::from_millis(250)
        );
        assert_eq!(
            Time::<Fixed>::from_seconds(0.5).timestep(),
            Duration::from_millis(500)
        );
        assert_eq!(
            Time::<Fixed>::from_duration(Duration::from_secs(1)).timestep(),
            Duration::from_secs(1)
        );
    }

    #[test]
    #[should_panic(expected = "fixed timestep to zero")]
    fn test_zero_timestep_panics() {
        Time::<Fixed>::from_duration(Duration::ZERO);
    }

    #[test]
    fn test_expend_partial_and_multiple() {
        let mut time = Time::<Fixed>::from_seconds(2.0);

        time.accumulate_overstep(Duration::from_secs(1));
        assert!(!time.expend()); // less than one step accumulated
        assert_eq!(time.elapsed(), Duration::ZERO);
        assert_eq!(time.overstep(), Duration::from_secs(1));
        assert_eq!(time.overstep_fraction(), 0.5);

        time.accumulate_overstep(Duration::from_secs(6));
        assert!(time.expend());
        assert!(time.expend());
        assert!(time.expend());
        assert_eq!(time.delta(), Duration::from_secs(2));
        assert_eq!(time.elapsed(), Duration::from_secs(6));
        assert_eq!(time.overstep(), Duration::from_secs(1));
        assert!(!time.expend());

        time.discard_overstep(Duration::from_secs(10));
        assert_eq!(time.overstep(), Duration::ZERO);
    }

    #[test]
    fn test_run_fixed_main_schedule_zero_one_many() {
        use std::sync::{Arc, Mutex};

        let mut world = World::new();
        world.insert_resource(Time::<Virtual>::default());
        world.insert_resource(Time::<Fixed>::from_hz(2.0)); // 500 ms steps
        world.insert_resource(Time::<()>::default());
        let runs = Arc::new(Mutex::new(0u32));

        // 400 ms of virtual time: no full step.
        world
            .get_resource_mut::<Time<Virtual>>()
            .unwrap()
            .advance_by(Duration::from_millis(400));
        run_fixed_main_schedule(&mut world, |_| {
            *runs.lock().unwrap() += 1;
        });
        assert_eq!(*runs.lock().unwrap(), 0);

        // +200 ms → 600 ms accumulated: exactly one step, generic Time is the
        // fixed clock during the run.
        world
            .get_resource_mut::<Time<Virtual>>()
            .unwrap()
            .advance_by(Duration::from_millis(200));
        let runs_in = runs.clone();
        run_fixed_main_schedule(&mut world, move |world| {
            *runs_in.lock().unwrap() += 1;
            let generic = world.get_resource::<Time>().unwrap();
            assert_eq!(generic.delta(), Duration::from_millis(500));
            assert_eq!(generic.elapsed(), Duration::from_millis(500));
        });
        assert_eq!(*runs.lock().unwrap(), 1);
        // Generic Time is restored to the virtual clock afterwards.
        assert_eq!(
            world.get_resource::<Time>().unwrap().delta(),
            world.get_resource::<Time<Virtual>>().unwrap().delta()
        );

        // +1.05 s → 1.15 s accumulated: two more steps.
        world
            .get_resource_mut::<Time<Virtual>>()
            .unwrap()
            .advance_by(Duration::from_millis(1050));
        run_fixed_main_schedule(&mut world, |_| {
            *runs.lock().unwrap() += 1;
        });
        assert_eq!(*runs.lock().unwrap(), 3);
        // 150 ms remains toward the next step.
        assert_eq!(
            world.get_resource::<Time<Fixed>>().unwrap().overstep(),
            Duration::from_millis(150)
        );
    }

    #[test]
    fn test_run_fixed_main_schedule_no_clocks_is_noop() {
        let mut world = World::new();
        let mut ran = false;
        run_fixed_main_schedule(&mut world, |_| ran = true);
        assert!(!ran);
    }
}

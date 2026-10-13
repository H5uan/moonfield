//! Time resources for moonfield, ported from the reference implementation's
//! time crate (architecture-level; no reflection, single-threaded).
//!
//! Four clocks, all plain world resources:
//!
//! - [`Time<Real>`] — wall-clock time, fed [`Instant`]s once per frame by the
//!   [`time_update_system`]. Unaffected by pause/scaling.
//! - [`Time<Virtual>`] — game time, advanced from the real delta with pause,
//!   relative speed, and a per-update `max_delta` clamp applied.
//! - [`Time<Fixed>`] — fixed-timestep time for the fixed schedules;
//!   [`run_fixed_main_schedule`] accumulates the virtual delta and spends it
//!   in [`Time<Fixed>::timestep`] increments.
//! - [`Time`] — the generic "current" clock systems read via `Res<Time>`;
//!   refreshed from [`Time<Virtual>`] every frame, and mirrored to
//!   [`Time<Fixed>`] while the fixed schedules run.
//!
//! `TimePlugin` (moonfield-app) inserts the resources and registers
//! [`time_update_system`] in the `First` schedule, so `App::update` advances
//! the clocks automatically. The [`TimeUpdateStrategy`] resource selects the
//! source: the system clock (`Automatic`, the default) or deterministic
//! values for tests (`ManualInstant`, `ManualDuration`, `FixedTimesteps`).
//! `update_time`/`update_time_with_instant` remain callable directly for
//! one-off tests; missing clocks are lazily inserted, so apps that skip the
//! plugin still get working time.

use std::time::{Duration, Instant};

use moonfield_ecs::World;

mod fixed;
mod real;
mod time;
mod virt;

pub use fixed::{Fixed, run_fixed_main_schedule};
pub use real::Real;
pub use time::Time;
pub use virt::{Virtual, update_virtual_time};

/// Common time imports.
pub mod prelude {
    pub use crate::{Fixed, Real, Time, Virtual};
}

/// Configuration resource used to determine how the time system should run,
/// mirroring Bevy's `TimeUpdateStrategy`.
///
/// `Automatic` (the default) is fine for normal use; tests, networking, and
/// replay scenarios set a deterministic strategy instead.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TimeUpdateStrategy {
    /// Update the clocks from the system clock each frame.
    #[default]
    Automatic,
    /// Set the real clock to the specified [`Instant`] each frame. To make
    /// time progress, this value must be manually updated each frame.
    ManualInstant(Instant),
    /// Increment the real clock by the specified [`Duration`] each frame.
    ManualDuration(Duration),
    /// Increment the real clock by the fixed timestep times `n` each frame,
    /// so `App::update` always runs the fixed loop exactly `n` times.
    FixedTimesteps(u32),
}

/// The `First` schedule system that advances all clocks according to the
/// [`TimeUpdateStrategy`] resource (Bevy's `time_system`). Registered by
/// `TimePlugin`; runs before the fixed loop and `Update`.
pub fn time_update_system(world: &mut World) {
    let strategy = world
        .get_resource::<TimeUpdateStrategy>()
        .map(|strategy| *strategy)
        .unwrap_or_default();
    match strategy {
        TimeUpdateStrategy::Automatic => update_time(world),
        TimeUpdateStrategy::ManualInstant(instant) => update_time_with_instant(world, instant),
        TimeUpdateStrategy::ManualDuration(duration) => update_time_with_duration(world, duration),
        TimeUpdateStrategy::FixedTimesteps(factor) => {
            let step = world
                .get_resource::<Time<Fixed>>()
                .map(|fixed| fixed.timestep())
                .unwrap_or_else(|| Time::<Fixed>::default().timestep());
            update_time_with_duration(world, step * factor);
        }
    }
}

/// Advances all time resources from [`Instant::now`]. Called by
/// [`time_update_system`] under `Automatic` strategy.
pub fn update_time(world: &mut World) {
    update_time_with_instant(world, Instant::now());
}

/// Advances all time resources from a specific [`Instant`]
/// (real → virtual → generic). This is the testable form of
/// [`update_time`]; missing clocks are lazily inserted with defaults, so the
/// function works whether or not `TimePlugin` was added.
pub fn update_time_with_instant(world: &mut World, instant: Instant) {
    ensure_clocks(world);
    world
        .get_resource_mut::<Time<Real>>()
        .unwrap()
        .update_with_instant(instant);
    refresh_virtual_time(world);
}

/// Advances all time resources as if `duration` had passed since the last
/// update (real → virtual → generic). The deterministic counterpart of
/// [`update_time_with_instant`] used by the `ManualDuration` and
/// `FixedTimesteps` strategies.
pub fn update_time_with_duration(world: &mut World, duration: Duration) {
    ensure_clocks(world);
    world
        .get_resource_mut::<Time<Real>>()
        .unwrap()
        .update_with_duration(duration);
    refresh_virtual_time(world);
}

/// Lazily insert the three clocks `update_time_with_*` drive, so the
/// functions work whether or not `TimePlugin` was added.
fn ensure_clocks(world: &mut World) {
    if !world.contains_resource::<Time<Real>>() {
        world.insert_resource(Time::<Real>::default());
    }
    if !world.contains_resource::<Time<Virtual>>() {
        world.insert_resource(Time::<Virtual>::default());
    }
    if !world.contains_resource::<Time>() {
        world.insert_resource(Time::<()>::default());
    }
}

/// Push the real clock's delta through the virtual clock (pause/speed/clamp)
/// and refresh the generic `Time` resource from it.
fn refresh_virtual_time(world: &mut World) {
    let real = world.get_resource::<Time<Real>>().unwrap();
    let mut virt = world.get_resource_mut::<Time<Virtual>>().unwrap();
    let mut current = world.get_resource_mut::<Time>().unwrap();
    update_virtual_time(&mut current, &mut virt, &real);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_update_time_with_instant_advances_all_clocks() {
        let mut world = World::new();
        let t0 = Instant::now();

        // First frame: zero delta everywhere (no previous update instant).
        update_time_with_instant(&mut world, t0);
        assert_eq!(
            world.get_resource::<Time<Real>>().unwrap().delta(),
            Duration::ZERO
        );
        assert_eq!(
            world.get_resource::<Time>().unwrap().delta(),
            Duration::ZERO
        );

        // Second frame: real, virtual, and generic clocks all see 16 ms.
        update_time_with_instant(&mut world, t0 + Duration::from_millis(16));
        assert_eq!(
            world.get_resource::<Time<Real>>().unwrap().delta(),
            Duration::from_millis(16)
        );
        assert_eq!(
            world.get_resource::<Time<Virtual>>().unwrap().delta(),
            Duration::from_millis(16)
        );
        let current = world.get_resource::<Time>().unwrap();
        assert_eq!(current.delta(), Duration::from_millis(16));
        assert_eq!(current.elapsed(), Duration::from_millis(16));
    }

    #[test]
    fn test_update_time_respects_pause_and_speed() {
        let mut world = World::new();
        let t0 = Instant::now();
        update_time_with_instant(&mut world, t0);

        world
            .get_resource_mut::<Time<Virtual>>()
            .unwrap()
            .set_relative_speed(2.0);
        update_time_with_instant(&mut world, t0 + Duration::from_millis(16));
        // Virtual time (and the generic clock) run at double speed…
        assert_eq!(
            world.get_resource::<Time<Virtual>>().unwrap().delta(),
            Duration::from_millis(32)
        );
        // …while the real clock is unaffected.
        assert_eq!(
            world.get_resource::<Time<Real>>().unwrap().delta(),
            Duration::from_millis(16)
        );

        world.get_resource_mut::<Time<Virtual>>().unwrap().pause();
        update_time_with_instant(&mut world, t0 + Duration::from_millis(32));
        assert_eq!(
            world.get_resource::<Time<Virtual>>().unwrap().delta(),
            Duration::ZERO
        );
        assert_eq!(
            world.get_resource::<Time>().unwrap().delta(),
            Duration::ZERO
        );
        assert_eq!(
            world.get_resource::<Time<Real>>().unwrap().delta(),
            Duration::from_millis(16)
        );
    }
}

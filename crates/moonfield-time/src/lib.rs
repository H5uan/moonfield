//! Time resources for moonfield, ported from the reference implementation's
//! time crate (architecture-level; no reflection, single-threaded).
//!
//! Four clocks, all plain world resources:
//!
//! - [`Time<Real>`] — wall-clock time, fed [`Instant`]s once per frame by the
//!   windowing backend. Unaffected by pause/scaling.
//! - [`Time<Virtual>`] — game time, advanced from the real delta with pause,
//!   relative speed, and a per-update `max_delta` clamp applied.
//! - [`Time<Fixed>`] — fixed-timestep time for the fixed schedules;
//!   [`run_fixed_main_schedule`] accumulates the virtual delta and spends it
//!   in [`Time<Fixed>::timestep`] increments.
//! - [`Time`] — the generic "current" clock systems read via `Res<Time>`;
//!   refreshed from [`Time<Virtual>`] every frame, and mirrored to
//!   [`Time<Fixed>`] while the fixed schedules run.
//!
//! `TimePlugin` (moonfield-app) inserts the resources; the winit backend
//! calls [`update_time`] at the start of every frame, before
//! `App::update`. `update_time` also lazily inserts any missing clock, so
//! apps that skip the plugin (e.g. the editor path) still get working time.
//!
//! # Deferred
//!
//! The reference's `Timer`/`Stopwatch` and `TimeUpdateStrategy` are not
//! ported — tests drive the clocks through [`update_time_with_instant`]
//! instead.

use std::time::Instant;

use moonfield_ecs::World;

mod fixed;
mod real;
mod time;
mod virt;

pub use fixed::{run_fixed_main_schedule, Fixed};
pub use real::Real;
pub use time::Time;
pub use virt::{update_virtual_time, Virtual};

/// Common time imports.
pub mod prelude {
    pub use crate::{Fixed, Real, Time, Virtual};
}

/// Advances all time resources from [`Instant::now`]. Called by the windowing
/// backend once per frame, before the app update.
pub fn update_time(world: &mut World) {
    update_time_with_instant(world, Instant::now());
}

/// Advances all time resources from a specific [`Instant`]
/// (real → virtual → generic). This is the testable form of
/// [`update_time`]; missing clocks are lazily inserted with defaults, so the
/// function works whether or not `TimePlugin` was added.
pub fn update_time_with_instant(world: &mut World, instant: Instant) {
    if !world.contains_resource::<Time<Real>>() {
        world.insert_resource(Time::<Real>::default());
    }
    if !world.contains_resource::<Time<Virtual>>() {
        world.insert_resource(Time::<Virtual>::default());
    }
    if !world.contains_resource::<Time>() {
        world.insert_resource(Time::<()>::default());
    }
    world
        .get_resource_mut::<Time<Real>>()
        .unwrap()
        .update_with_instant(instant);
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

//! Plugin wiring the time resources into an [`App`].
//!
//! The clocks themselves live in `moonfield-time` (dependency direction:
//! app → time, so [`App::update`] can drive the fixed-timestep loop); the
//! composition plugin follows the `HierarchyPlugin` pattern.

use crate::{App, First, Plugin};
use moonfield_time::{Fixed, Real, Time, TimeUpdateStrategy, Virtual};

/// Inserts the [`Time`] / [`Time<Real>`] / [`Time<Virtual>`] /
/// [`Time<Fixed>`] resources with defaults (existing resources are left
/// untouched) and registers [`time_update_system`] in `First`, so every
/// [`App::update`] advances the clocks per the [`TimeUpdateStrategy`]
/// resource. The fixed-schedule loop is driven by [`App::update`], not this
/// plugin.
///
/// Without this plugin the clocks are lazily inserted by
/// `moonfield-time::update_time_with_*` and the fixed schedules never run.
pub struct TimePlugin;

impl Plugin for TimePlugin {
    fn name(&self) -> &str {
        "moonfield_app::TimePlugin"
    }

    fn build(&self, app: &mut App) {
        let world = app.world_mut();
        if !world.contains_resource::<Time<Real>>() {
            world.insert_resource(Time::<Real>::default());
        }
        if !world.contains_resource::<Time<Virtual>>() {
            world.insert_resource(Time::<Virtual>::default());
        }
        if !world.contains_resource::<Time<Fixed>>() {
            world.insert_resource(Time::<Fixed>::default());
        }
        if !world.contains_resource::<Time>() {
            world.insert_resource(Time::<()>::default());
        }
        if !world.contains_resource::<TimeUpdateStrategy>() {
            world.insert_resource(TimeUpdateStrategy::default());
        }
        app.add_systems(First, moonfield_time::time_update_system);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_plugin_inserts_resources() {
        let mut app = App::new();
        app.add_plugin(TimePlugin);
        assert!(app.world().contains_resource::<Time>());
        assert!(app.world().contains_resource::<Time<Real>>());
        assert!(app.world().contains_resource::<Time<Virtual>>());
        assert!(app.world().contains_resource::<Time<Fixed>>());
    }
}

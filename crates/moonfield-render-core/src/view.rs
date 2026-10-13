//! Per-view execution: the camera driver and the resource per-view
//! systems read.

use crate::scene::ExtractedView;
use moonfield_ecs::{Entity, ScheduleLabel, World};

/// The view a per-view schedule is currently running for. [`camera_driver`]
/// inserts it for each view and removes it after the loop.
pub struct CurrentView(pub Entity);

/// Order the extracted views by `(Camera::order, entity)` and run the `L`
/// schedule once per view, [`CurrentView`] pointing at it. Register one
/// instantiation per view schedule — the label names the schedule the
/// driver runs.
pub fn camera_driver<L: ScheduleLabel + Default>(world: &mut World) {
    let mut views: Vec<(f32, Entity)> = world
        .query::<&ExtractedView>()
        .map(|(entity, view)| (view.camera.order, entity))
        .collect();
    views.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1.to_bits().cmp(&b.1.to_bits()))
    });

    for (_, entity) in views {
        world.insert_resource(CurrentView(entity));
        world.run_schedule(L::default());
    }
    world.remove_resource::<CurrentView>();
}

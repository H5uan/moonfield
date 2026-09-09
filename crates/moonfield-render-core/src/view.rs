//! Per-view execution: the camera driver and the parameter per-view
//! systems read.

use crate::scene::ExtractedView;
use moonfield_ecs::{Component, Entity, ScheduleLabel, SystemParam, World};
use std::ops::Deref;

/// The view a per-view schedule is currently running for. [`camera_driver`]
/// inserts it for each view and removes it after the loop.
pub struct CurrentView(pub Entity);

/// System parameter fetching `T` from the current view entity. Only
/// fetchable inside a per-view schedule run — the camera driver points
/// [`CurrentView`] at each view before running the schedule.
///
/// Single-component, read-only — the shape per-entity access supports today;
/// tuple support lands with a caller that needs it.
pub struct ViewQuery<'w, T: Component>(Option<&'w T>);

impl<T: Component> SystemParam for ViewQuery<'_, T> {
    type State = ();
    type Item<'w, 's> = ViewQuery<'w, T>;

    fn init_state() -> Self::State {}

    fn fetch<'w, 's>(world: &'w World, _state: &'s mut Self::State) -> Self::Item<'w, 's> {
        let view = world.get_resource::<CurrentView>().map(|view| view.0);
        ViewQuery(view.and_then(|entity| world.get_component::<T>(entity)))
    }
}

impl<'w, T: Component> Deref for ViewQuery<'w, T> {
    type Target = Option<&'w T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

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

//! Extraction systems: per-frame copies from the main world into the render
//! world, run by the `ExtractSchedule` (see `App::render`).
//!
//! The render world's entities are cleared before the schedule runs, so
//! every system here rebuilds its full set each frame. Extraction is
//! one-way: systems read the main world through the [`Extract`] parameter
//! and write the render world through deferred [`Commands`], applied after
//! each system so later extract systems observe earlier ones' spawns.
//! Nothing may key cross-frame state by the entities spawned here — they
//! are rebuilt every frame.

use crate::scene::{ExtractedView, ViewTarget};
use crate::window::WindowFrameDemand;
use moonfield_camera::{Camera, CameraTarget, PrimaryCamera, RenderTarget};
use moonfield_ecs::{Commands, Component, MainWorld, Query, SystemParam, World};
use moonfield_math::GlobalTransform;
use std::cell::Ref;
use std::ops::{Deref, DerefMut};

/// The source entity in the main world for an extracted render-world entity.
///
/// Render-world entities are rebuilt every frame, so cross-world identity is
/// expressed with this component instead of a render-world [`moonfield_ecs::Entity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MainEntity(pub moonfield_ecs::Entity);

/// System parameter that fetches `T` from the main world instead of the
/// render world.
///
/// Only fetchable while the `ExtractSchedule` runs: `App::render` parks the
/// main world in the render world's [`MainWorld`] resource for the
/// schedule's duration, and `Extract` reads through it.
pub struct Extract<'w, 's, T: SystemParam> {
    /// The resource cell's shared borrow, held for the item's lifetime so
    /// the parked world cannot be replaced under `param`.
    _main: Ref<'w, MainWorld>,
    param: T::Item<'w, 's>,
}

impl<'w, 's, T: SystemParam> SystemParam for Extract<'w, 's, T> {
    type State = T::State;
    type Item<'wi, 'si> = Extract<'wi, 'si, T>;

    fn init_state() -> Self::State {
        T::init_state()
    }

    fn fetch<'wi, 'si>(world: &'wi World, state: &'si mut Self::State) -> Self::Item<'wi, 'si> {
        let main = world.get_resource::<MainWorld>().unwrap_or_else(|| {
            panic!(
                "`Extract` is only fetchable while `ExtractSchedule` runs \
                 (App::render parks the main world)"
            )
        });
        // SAFETY: `main` holds the resource cell's shared borrow for as long
        // as the returned item lives (the `_main` field), so the parked world
        // cannot be replaced while `param` borrows into it; the pointer is
        // valid for the schedule's duration by `MainWorld`'s contract.
        let main_world: &'wi World = unsafe { main.world() };
        let param = T::fetch(main_world, state);
        Extract { _main: main, param }
    }
}

impl<'w, 's, T: SystemParam> Deref for Extract<'w, 's, T> {
    type Target = T::Item<'w, 's>;

    fn deref(&self) -> &Self::Target {
        &self.param
    }
}

impl<'w, 's, T: SystemParam> DerefMut for Extract<'w, 's, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.param
    }
}

/// Copies every camera — `Camera` + `GlobalTransform`, plus the
/// `PrimaryCamera` marker when present — into the render world.
///
/// Also writes the base [`WindowFrameDemand`]: a camera targeting the primary
/// window demands window frames. Later extract systems with their own content
/// (the editor's UI) OR their demand in.
// The 4-tuple Option query is the system's real shape (moonfield-ecs carries
// the same allowance for system types).
#[allow(clippy::type_complexity)]
pub fn extract_cameras(
    cameras: Extract<
        Query<(
            &Camera,
            &GlobalTransform,
            Option<&CameraTarget>,
            Option<&PrimaryCamera>,
        )>,
    >,
    commands: Commands,
) {
    let mut window_demand = false;
    for (entity, (camera, global, target, primary)) in cameras.iter() {
        let (camera, global) = (*camera, *global);
        let target = target.copied().unwrap_or_default();
        window_demand |= matches!(target.0, RenderTarget::PrimaryWindow);
        let extracted_view = ExtractedView {
            main_entity: MainEntity(entity),
            camera,
            world_from_view: global,
            target: ViewTarget(target.0),
        };
        if primary.is_some() {
            commands.spawn((
                camera,
                global,
                MainEntity(entity),
                extracted_view,
                PrimaryCamera,
            ));
        } else {
            commands.spawn((camera, global, MainEntity(entity), extracted_view));
        }
    }
    commands.insert_resource(WindowFrameDemand(window_demand));
}

/// Copies every entity with component `T` + `GlobalTransform` into the
/// render world. Generic over the renderable component type, so feature
/// crates register one instantiation per component instead of hand-writing
/// an extraction system each.
pub fn extract_with_transform<T: Component + Copy>(
    entities: Extract<Query<(&T, &GlobalTransform)>>,
    commands: Commands,
) {
    for (entity, (component, global)) in entities.iter() {
        commands.spawn((*component, *global, MainEntity(entity)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_app::{App, ExtractSchedule, HierarchyPlugin, PreRender};
    use moonfield_ecs::{IntoSystemConfigs, ensure_global_transforms};
    use moonfield_math::{Transform, Vec3};

    /// Spawns two cameras in the main world (the first primary) and renders
    /// one frame; returns the app holding both worlds.
    fn app_with_two_cameras() -> App {
        let mut app = App::new();
        app.add_render_systems(ExtractSchedule, extract_cameras);
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::from(Transform::from_xyz(0.0, 1.0, 5.0)),
            PrimaryCamera,
        ));
        app.world_mut().spawn((
            Camera {
                fov_y_radians: 1.0,
                ..Camera::default()
            },
            GlobalTransform::IDENTITY,
        ));
        app.render();
        app
    }

    #[test]
    fn extracts_all_cameras_with_primary_marker() {
        let app = app_with_two_cameras();
        let render_world = app.render_world();

        let cameras: Vec<_> = render_world
            .query::<(&Camera, &GlobalTransform)>()
            .collect();
        assert_eq!(cameras.len(), 2);

        let primary: Vec<_> = render_world.query::<&PrimaryCamera>().collect();
        assert_eq!(primary.len(), 1);

        let extracted_sources: Vec<_> = render_world
            .query::<&MainEntity>()
            .map(|(_, main_entity)| main_entity.0)
            .collect();
        let main_sources: Vec<_> = app
            .world()
            .query::<&Camera>()
            .map(|(entity, _)| entity)
            .collect();
        assert_eq!(extracted_sources.len(), main_sources.len());
        assert!(
            main_sources
                .iter()
                .all(|entity| extracted_sources.contains(entity))
        );

        // The copied values match the main world's cameras.
        let main_cameras: Vec<Camera> = app
            .world()
            .query::<&Camera>()
            .map(|(_, camera)| *camera)
            .collect();
        let render_cameras: Vec<Camera> = cameras.iter().map(|(_, (camera, _))| **camera).collect();
        for camera in &main_cameras {
            assert!(render_cameras.contains(camera));
        }
    }

    #[test]
    fn extraction_rebuilds_every_frame() {
        let mut app = app_with_two_cameras();

        // Rendering again must not accumulate duplicates.
        app.render();
        assert_eq!(app.render_world().query::<&Camera>().count(), 2);

        // A camera despawned from the main world disappears from the render
        // world on the next frame.
        let entity = app.world_mut().query::<&PrimaryCamera>().next().unwrap().0;
        app.world_mut().despawn(entity).unwrap();
        app.render();
        assert_eq!(app.render_world().query::<&Camera>().count(), 1);
        assert_eq!(app.render_world().query::<&PrimaryCamera>().count(), 0);
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct SpinningCube {
        speed: f32,
    }

    #[test]
    fn window_targeted_camera_demands_window_frames() {
        use crate::window::WindowFrameDemand;
        use moonfield_camera::RenderTarget;

        let mut app = App::new();
        app.add_render_systems(ExtractSchedule, extract_cameras);
        app.world_mut().spawn((
            Camera::default(),
            GlobalTransform::IDENTITY,
            PrimaryCamera,
            CameraTarget(RenderTarget::PrimaryWindow),
        ));
        app.render();
        assert_eq!(
            app.render_world()
                .get_resource::<WindowFrameDemand>()
                .map(|demand| demand.0),
            Some(true)
        );
    }

    #[test]
    fn viewport_camera_demands_no_window_frames() {
        use crate::window::WindowFrameDemand;

        let mut app = App::new();
        app.add_render_systems(ExtractSchedule, extract_cameras);
        // No CameraTarget: the default target is the offscreen viewport.
        app.world_mut()
            .spawn((Camera::default(), GlobalTransform::IDENTITY));
        app.render();
        assert_eq!(
            app.render_world()
                .get_resource::<WindowFrameDemand>()
                .map(|demand| demand.0),
            Some(false)
        );
    }

    #[test]
    fn extract_with_transform_copies_pairs_only() {
        let mut app = App::new();
        app.add_render_systems(ExtractSchedule, extract_with_transform::<SpinningCube>);
        // Component + transform: extracted.
        app.world_mut()
            .spawn((SpinningCube { speed: 1.0 }, GlobalTransform::IDENTITY));
        // Missing the component or the transform: not extracted.
        app.world_mut().spawn((GlobalTransform::IDENTITY,));
        app.world_mut().spawn((SpinningCube { speed: 2.0 },));
        app.render();

        let extracted: Vec<_> = app
            .render_world()
            .query::<(&SpinningCube, &GlobalTransform)>()
            .collect();
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].1.0.speed, 1.0);
        let source = app
            .render_world()
            .get_component::<MainEntity>(extracted[0].0)
            .unwrap()
            .0;
        assert_eq!(
            app.world()
                .get_component::<SpinningCube>(source)
                .unwrap()
                .speed,
            1.0
        );
    }

    #[derive(Debug, Clone, Copy)]
    struct ExtractedTranslation(Vec3);

    #[test]
    fn pre_render_transform_changes_are_visible_to_extraction() {
        fn move_camera(world: &mut World) {
            let entity = world.query::<&Transform>().next().unwrap().0;
            *world.get_component_mut::<Transform>(entity).unwrap() =
                Transform::from_xyz(3.0, 2.0, 1.0);
        }

        fn extract_globals(globals: Extract<Query<&GlobalTransform>>, commands: Commands) {
            for (_, global) in globals.iter() {
                commands.spawn((ExtractedTranslation(global.translation()),));
            }
        }

        let mut app = App::new();
        app.add_plugin(HierarchyPlugin);
        app.world_mut().spawn((Transform::IDENTITY,));
        app.add_systems(PreRender, move_camera.before(&ensure_global_transforms));
        app.add_render_systems(ExtractSchedule, extract_globals);

        app.render();

        let (_, extracted) = app
            .render_world()
            .query::<&ExtractedTranslation>()
            .next()
            .expect("the propagated transform should be extracted");
        assert!((extracted.0 - Vec3::new(3.0, 2.0, 1.0)).length() < 1e-5);
    }
}

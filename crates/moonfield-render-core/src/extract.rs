//! Handwritten extraction functions: per-frame copies from the main world
//! into the render world (see `App::add_extract_system`).
//!
//! The render world's entities are cleared before extraction runs, so every
//! function here rebuilds its full set each frame. Extraction is a one-way,
//! read-only snapshot: functions take the main world as `&World` and must
//! not mutate it, and nothing in the render world may key cross-frame state
//! by the entities spawned here — they are rebuilt every frame.

use crate::scene::{ExtractedView, ViewTarget};
use moonfield_camera::{Camera, CameraTarget, PrimaryCamera};
use moonfield_ecs::{Component, World};
use moonfield_math::GlobalTransform;

/// The source entity in the main world for an extracted render-world entity.
///
/// Render-world entities are rebuilt every frame, so cross-world identity is
/// expressed with this component instead of a render-world [`moonfield_ecs::Entity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MainEntity(pub moonfield_ecs::Entity);

/// Copies every camera — `Camera` + `GlobalTransform`, plus the
/// `PrimaryCamera` marker when present — into the render world.
pub fn extract_cameras(world: &World, render_world: &mut World) {
    for (entity, (camera, global)) in world.query::<(&Camera, &GlobalTransform)>() {
        let (camera, global) = (*camera, *global);
        let target = world
            .get_component::<CameraTarget>(entity)
            .copied()
            .unwrap_or_default();
        let extracted_view = ExtractedView {
            main_entity: MainEntity(entity),
            camera,
            world_from_view: global,
            target: ViewTarget(target.0),
        };
        if world.get_component::<PrimaryCamera>(entity).is_some() {
            render_world.spawn((
                camera,
                global,
                MainEntity(entity),
                extracted_view,
                PrimaryCamera,
            ));
        } else {
            render_world.spawn((camera, global, MainEntity(entity), extracted_view));
        }
    }
}

/// Copies every entity with component `T` + `GlobalTransform` into the
/// render world. Generic over the renderable component type, so feature
/// crates register one instantiation per component instead of hand-writing
/// an extraction function each.
pub fn extract_with_transform<T: Component + Copy>(world: &World, render_world: &mut World) {
    for (entity, (component, global)) in world.query::<(&T, &GlobalTransform)>() {
        render_world.spawn((*component, *global, MainEntity(entity)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_app::App;
    use moonfield_math::Transform;

    /// Spawns two cameras in the main world (the first primary) and renders
    /// one frame; returns the app holding both worlds.
    fn app_with_two_cameras() -> App {
        let mut app = App::new();
        app.add_extract_system(extract_cameras);
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
    fn extract_with_transform_copies_pairs_only() {
        let mut app = App::new();
        app.add_extract_system(extract_with_transform::<SpinningCube>);
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
}

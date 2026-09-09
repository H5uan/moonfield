//! The core 3D view schedule: per-view phases and the opaque pass.

pub mod pass;

use moonfield_app::prelude::{ScheduleLabel, SystemSet, World};
use moonfield_render_core::{ExtractedView, RenderPhase};

use crate::render_phase::Opaque3d;

/// Schedule label for the per-view 3D systems; the camera driver runs it
/// once per extracted view.
#[derive(Default)]
pub struct Core3d;

impl ScheduleLabel for Core3d {}

/// The `Core3d` schedule's opaque-pass anchor: everything recording 3D
/// geometry into the view's target attaches around it.
pub struct Core3dOpaquePass;

impl SystemSet for Core3dOpaquePass {}

/// `Queue` system: attach an empty opaque phase to every extracted view;
/// feature queue systems fill the phases afterwards.
pub fn prepare_view_phases(world: &mut World) {
    let views: Vec<_> = world
        .query::<&ExtractedView>()
        .map(|(entity, _)| entity)
        .collect();
    for entity in views {
        world.insert_component(entity, RenderPhase::<Opaque3d>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenderFeaturePlugin, mesh::Mesh, mesh::MeshHandle, mesh::MeshRenderer};
    use moonfield_app::{App, ExtractSchedule};
    use moonfield_asset::Assets;
    use moonfield_camera::{Camera, CameraTarget, PrimaryCamera, RenderTarget};
    use moonfield_math::{GlobalTransform, Transform};
    use moonfield_render_core::extract_cameras;

    #[test]
    fn test_every_view_gets_an_isolated_phase() {
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        app.add_render_systems(ExtractSchedule, extract_cameras);
        let mesh = {
            app.world()
                .get_resource_mut::<Assets<Mesh>>()
                .unwrap()
                .add(Mesh::new(vec![[0.0; 3]], vec![0], None))
        };
        app.world_mut()
            .spawn((Camera::default(), PrimaryCamera, GlobalTransform::IDENTITY));
        app.world_mut().spawn((
            Camera::default(),
            CameraTarget(RenderTarget::PrimaryWindow),
            GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 5.0)),
        ));
        app.world_mut().spawn((
            MeshRenderer::new(MeshHandle(mesh), [1.0; 4]),
            GlobalTransform::from(Transform::from_xyz(0.0, 0.0, -2.0)),
        ));

        app.render();
        let views: Vec<_> = app
            .render_world()
            .query::<(&ExtractedView, &RenderPhase<Opaque3d>)>()
            .collect();
        assert_eq!(views.len(), 2);
        for (_, (_, phase)) in &views {
            assert_eq!(phase.items().len(), 1);
        }
    }
}

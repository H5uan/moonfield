//! Camera-driven construction of 3D render phases.

pub mod pass;

use moonfield_app::prelude::World;
use moonfield_camera::PrimaryCamera;
use moonfield_render_core::{ExtractedView, MainEntity, RenderPhase, ViewTarget};

use crate::render_phase::Opaque3d;

/// Render work associated with one extracted camera.
#[derive(Debug, Clone, PartialEq)]
pub struct Core3dView {
    /// Source camera entity.
    pub main_entity: MainEntity,
    /// Logical output selected by the camera.
    pub target: ViewTarget,
    /// Extracted camera data.
    pub view: ExtractedView,
    /// Whether the source camera carries [`PrimaryCamera`].
    pub is_primary: bool,
    /// Opaque mesh items visible to this view, filled by
    /// [`crate::render_phase::queue_opaque_3d`].
    pub opaque: RenderPhase<Opaque3d>,
}

/// Per-frame 3D work produced by the camera driver.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Core3dFrame {
    views: Vec<Core3dView>,
}

impl Core3dFrame {
    /// Build one empty view record per extracted camera. Feature queue
    /// systems (e.g. [`crate::render_phase::queue_opaque_3d`]) fill the
    /// per-view phases afterwards.
    pub fn build(world: &World) -> Self {
        let mut views = Vec::new();
        for (entity, view) in world.query::<&ExtractedView>() {
            views.push(Core3dView {
                main_entity: view.main_entity,
                target: view.target(),
                view: *view,
                is_primary: world.get_component::<PrimaryCamera>(entity).is_some(),
                opaque: RenderPhase::default(),
            });
        }
        views.sort_by_key(|view| !view.is_primary);
        Self { views }
    }

    /// Camera views, with primary views first.
    pub fn views(&self) -> &[Core3dView] {
        &self.views
    }

    /// Mutable camera views for queue systems filling per-view phases.
    pub fn views_mut(&mut self) -> &mut [Core3dView] {
        &mut self.views
    }

    /// First primary view targeting `target`.
    pub fn primary_view(&self, target: ViewTarget) -> Option<&Core3dView> {
        self.views
            .iter()
            .find(|view| view.is_primary && view.target == target)
    }
}

/// Rebuild the [`Core3dFrame`] resource from the current render-world snapshot.
pub fn prepare_core_3d_frame(world: &mut World) {
    let frame = Core3dFrame::build(world);
    world.insert_resource(frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh::Mesh, mesh::MeshHandle, mesh::MeshRenderer, RenderFeaturePlugin};
    use moonfield_app::App;
    use moonfield_asset::Assets;
    use moonfield_camera::{Camera, CameraTarget, RenderTarget};
    use moonfield_math::{GlobalTransform, Transform};
    use moonfield_render_core::extract_cameras;

    #[test]
    fn test_core_3d_builds_isolated_phases_for_each_camera() {
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        app.add_extract_system(extract_cameras);
        let mesh = {
            app.world()
                .get_resource_mut::<Assets<Mesh>>()
                .unwrap()
                .add(Mesh::new(vec![[0.0; 3]], vec![0], None))
        };
        let primary =
            app.world_mut()
                .spawn((Camera::default(), PrimaryCamera, GlobalTransform::IDENTITY));
        let secondary = app.world_mut().spawn((
            Camera::default(),
            CameraTarget(RenderTarget::PrimaryWindow),
            GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 5.0)),
        ));
        app.world_mut().spawn((
            MeshRenderer::new(MeshHandle(mesh), [1.0; 4]),
            GlobalTransform::from(Transform::from_xyz(0.0, 0.0, -2.0)),
        ));

        app.render();
        let frame = app
            .render_world()
            .get_resource::<Core3dFrame>()
            .expect("Core3dFrame");

        assert_eq!(frame.views().len(), 2);
        assert_eq!(frame.views()[0].main_entity.0, primary);
        assert_eq!(frame.views()[1].main_entity.0, secondary);
        assert_eq!(frame.views()[0].opaque.items().len(), 1);
        assert_eq!(frame.views()[1].opaque.items().len(), 1);
        assert_eq!(
            frame
                .primary_view(ViewTarget(RenderTarget::Viewport))
                .unwrap()
                .main_entity
                .0,
            primary
        );
    }
}

//! The renderer's runtime plugin: asset stores and extraction systems.

use moonfield_app::prelude::IntoSystemConfigs;
use moonfield_app::{App, Plugin, Render, RenderPrepare, RenderQueue};
use moonfield_render_core::{DrawFunctions, extract_with_transform};

use crate::mesh::{Mesh, MeshRenderer, PreparedGpuMeshes, extract_mesh_assets, prepare_meshes};
use crate::render_phase::{DrawMesh, Opaque3d, Opaque3dDrawFunction, queue_opaque_3d};
#[cfg(feature = "splat")]
use crate::splat::cloud::SplatCloud;

/// Registers the renderer's ECS surface: the `Assets` stores entities
/// reference through `MeshRenderer` / `SplatCloudHandle`, and the per-frame
/// extraction of those components into the render world.
pub struct RenderFeaturePlugin;

impl Plugin for RenderFeaturePlugin {
    fn name(&self) -> &str {
        "moonfield_render_feature::RenderFeaturePlugin"
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(moonfield_asset::Assets::<Mesh>::default());
        #[cfg(feature = "splat")]
        app.insert_resource(moonfield_asset::Assets::<SplatCloud>::default());

        app.add_extract_system(extract_mesh_assets);
        app.add_extract_system(extract_with_transform::<MeshRenderer>);
        app.render_world_mut()
            .insert_resource(crate::core_3d::Core3dFrame::default());
        app.render_world_mut()
            .insert_resource(PreparedGpuMeshes::default());
        let mut draw_functions = DrawFunctions::<Opaque3d>::default();
        let opaque_draw = draw_functions.register(DrawMesh);
        app.render_world_mut().insert_resource(draw_functions);
        app.render_world_mut()
            .insert_resource(Opaque3dDrawFunction(opaque_draw));
        app.add_render_systems(RenderPrepare, prepare_meshes);
        app.add_render_systems(RenderQueue, crate::core_3d::prepare_core_3d_frame);
        app.add_render_systems(
            RenderQueue,
            queue_opaque_3d.after(&crate::core_3d::prepare_core_3d_frame),
        );
        app.add_render_systems(
            Render,
            (
                crate::core_3d::pass::prepare_view_targets
                    .after(&moonfield_render_core::acquire_window_frames)
                    .before(&crate::core_3d::pass::main_opaque_pass_3d),
                crate::core_3d::pass::main_opaque_pass_3d
                    .after(&moonfield_render_core::acquire_window_frames)
                    .before(&moonfield_render_core::submit_window_frames),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_feature_plugin_registers_asset_stores() {
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        assert!(
            app.world()
                .contains_resource::<moonfield_asset::Assets<Mesh>>()
        );
        assert!(app.render_world().contains_resource::<PreparedGpuMeshes>());
        #[cfg(feature = "splat")]
        assert!(
            app.world()
                .contains_resource::<moonfield_asset::Assets<SplatCloud>>()
        );
    }

    #[test]
    fn test_render_feature_plugin_extracts_only_referenced_meshes_and_tracks_revisions() {
        use moonfield_math::GlobalTransform;

        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        let (referenced, unused) = {
            let mut meshes = app
                .world()
                .get_resource_mut::<moonfield_asset::Assets<Mesh>>()
                .unwrap();
            let referenced = meshes.add(Mesh::new(vec![[0.0; 3]], vec![0], None));
            let unused = meshes.add(Mesh::new(vec![[1.0; 3]], vec![0], None));
            (referenced, unused)
        };
        app.world_mut().spawn((
            MeshRenderer::new(crate::mesh::MeshHandle(referenced), [1.0; 4]),
            GlobalTransform::IDENTITY,
        ));

        app.render();
        let first_revision = app
            .render_world()
            .get_resource::<crate::mesh::ExtractedMeshes>()
            .unwrap()
            .get(referenced.id())
            .unwrap()
            .revision;
        assert!(
            app.render_world()
                .get_resource::<crate::mesh::ExtractedMeshes>()
                .unwrap()
                .get(unused.id())
                .is_none()
        );

        app.world()
            .get_resource_mut::<moonfield_asset::Assets<Mesh>>()
            .unwrap()
            .get_mut(&referenced)
            .unwrap();
        app.render();
        let second_revision = app
            .render_world()
            .get_resource::<crate::mesh::ExtractedMeshes>()
            .unwrap()
            .get(referenced.id())
            .unwrap()
            .revision;
        assert!(second_revision > first_revision);

        app.world()
            .get_resource_mut::<moonfield_asset::Assets<Mesh>>()
            .unwrap()
            .remove(&referenced)
            .unwrap();
        app.render();
        assert!(
            app.render_world()
                .get_resource::<crate::mesh::ExtractedMeshes>()
                .unwrap()
                .get(referenced.id())
                .is_none()
        );
    }

    #[test]
    fn test_render_feature_prepares_gpu_meshes_before_queueing() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let render_device = match moonfield_rhi::RenderDevice::new() {
            Ok(render_device) => render_device,
            Err(error) => {
                eprintln!("skipping: no Vulkan device available ({error})");
                return;
            }
        };
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        app.render_world_mut().insert_resource(render_device);
        let mesh = app
            .world()
            .get_resource_mut::<moonfield_asset::Assets<Mesh>>()
            .unwrap()
            .add(Mesh::new(
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                vec![0, 1, 2],
                None,
            ));
        app.world_mut().spawn((
            MeshRenderer::new(crate::mesh::MeshHandle(mesh), [1.0; 4]),
            moonfield_math::GlobalTransform::IDENTITY,
        ));

        app.render();

        let prepared = app
            .render_world()
            .get_resource::<PreparedGpuMeshes>()
            .unwrap();
        let gpu_mesh = prepared.get(mesh.id()).expect("mesh prepared before queue");
        assert_eq!(gpu_mesh.vertex().size(), 36);
        assert_eq!(gpu_mesh.index().size(), 12);
        assert_eq!(gpu_mesh.index_count(), 3);
    }
}

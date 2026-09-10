//! The renderer's runtime plugin: asset stores and extraction systems.

use moonfield_app::prelude::IntoSystemConfigs;
use moonfield_app::{App, ExtractSchedule, Plugin, Render};
use moonfield_render_core::camera_driver;
use moonfield_render_core::prepare_phase;
use moonfield_render_core::prepare_view_attachments;
use moonfield_render_core::schedule as render_sets;
use moonfield_render_core::{DrawFunctions, SortedPhasePlugin, extract_with_transform};

use crate::mesh::{Mesh, MeshRenderer, PreparedGpuMeshes, extract_mesh_assets, prepare_meshes};
use crate::render_phase::{DrawMesh, Opaque3d, queue_opaque_3d};
use crate::shader::{PipelineShaders, PreparedShaders, extract_shader_assets, prepare_shaders};
#[cfg(feature = "splat")]
use crate::splat::cloud::SplatCloud;

/// Registers the renderer's ECS surface: the `Assets` stores entities
/// reference through `MeshRenderer` / `SplatCloudHandle`, the shader-asset
/// pipeline (`PipelineShaders` requests → extraction → `PreparedShaders`),
/// and the per-frame extraction of those components into the render world.
pub struct RenderFeaturePlugin;

impl Plugin for RenderFeaturePlugin {
    fn name(&self) -> &str {
        "moonfield_render_feature::RenderFeaturePlugin"
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(moonfield_asset::Assets::<Mesh>::default());
        #[cfg(feature = "splat")]
        app.insert_resource(moonfield_asset::Assets::<SplatCloud>::default());
        app.insert_resource(moonfield_asset::Assets::<moonfield_shader::Shader>::default());
        app.insert_resource(PipelineShaders::default());

        app.add_render_systems(
            ExtractSchedule,
            (
                extract_mesh_assets,
                extract_shader_assets,
                extract_with_transform::<MeshRenderer>,
            ),
        );
        app.render_world_mut()
            .insert_resource(PreparedGpuMeshes::default());
        app.render_world_mut()
            .insert_resource(PreparedShaders::default());
        let mut draw_functions = DrawFunctions::<Opaque3d>::default();
        draw_functions.register::<DrawMesh>();
        app.render_world_mut().insert_resource(draw_functions);

        app.add_plugins(SortedPhasePlugin::<Opaque3d>::default());
        app.add_render_systems(
            Render,
            (prepare_meshes, prepare_shaders).in_set::<render_sets::PrepareAssets>(),
        );
        app.add_render_systems(
            Render,
            queue_opaque_3d
                .after(&prepare_phase::<Opaque3d>)
                .in_set::<render_sets::Queue>(),
        );
        app.add_render_systems(
            Render,
            (
                // The pooled offscreen targets must exist before render-core
                // resolves the per-view attachment components.
                crate::core_3d::pass::prepare_view_targets.before(&prepare_view_attachments),
                crate::core_3d::pass::prepare_core_3d_pipeline,
                crate::core_3d::pass::begin_frame_draw_arena,
            )
                .in_set::<render_sets::PrepareViews>(),
        );
        app.add_render_systems(
            Render,
            camera_driver::<crate::core_3d::Core3d>.in_set::<render_sets::CameraDriver>(),
        );
        // The per-view schedule: everything recording 3D geometry runs here,
        // once per view, anchored on the opaque pass.
        app.add_render_sets(crate::core_3d::Core3d, crate::core_3d::Core3dOpaquePass);
        app.add_render_systems(
            crate::core_3d::Core3d,
            crate::core_3d::pass::opaque_pass_3d.in_set::<crate::core_3d::Core3dOpaquePass>(),
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
        assert!(
            app.world()
                .contains_resource::<moonfield_asset::Assets<moonfield_shader::Shader>>()
        );
        assert!(
            app.world()
                .contains_resource::<crate::shader::PipelineShaders>()
        );
        assert!(app.render_world().contains_resource::<PreparedGpuMeshes>());
        assert!(
            app.render_world()
                .contains_resource::<crate::shader::PreparedShaders>()
        );
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
        // Insert the device first, mirroring the real plugin order
        // (`RenderPlugin` creates the `RenderDevice` before feature plugins
        // add their render-world resources): resources LIFO-drop before the
        // device, and the device's teardown guard stays on the clean path.
        app.render_world_mut().insert_resource(render_device);
        app.add_plugin(RenderFeaturePlugin);
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
        assert_ne!(gpu_mesh.positions().as_raw(), 0);
        assert_ne!(gpu_mesh.indices().as_raw(), 0);
        assert_eq!(gpu_mesh.index_count(), 3);
    }
}

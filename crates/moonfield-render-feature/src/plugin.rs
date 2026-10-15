//! The renderer's runtime plugin: asset stores and extraction systems.

use moonfield_app::prelude::IntoSystemConfigs;
use moonfield_app::{App, ExtractSchedule, Plugin, Render, Startup};
use moonfield_render_core::camera_driver;
use moonfield_render_core::prepare_phase;
use moonfield_render_core::prepare_view_attachments;
use moonfield_render_core::schedule as render_sets;
use moonfield_render_core::{DrawFunctions, SortedPhasePlugin, extract_with_transform};

use crate::mesh::{Mesh, MeshRenderer, PreparedGpuMeshes, extract_mesh_assets, prepare_meshes};
use crate::render_phase::{DrawMesh, Opaque3d, debug_scene_log, queue_opaque_3d};
use crate::shader::{
    PipelineShaders, PreparedShaders, extract_shader_assets, load_feature_shaders, prepare_shaders,
};
#[cfg(feature = "splat")]
use crate::splat::cloud::SplatCloud;

/// `Startup` system: the core 3D pass's shader is the mesh feature's own
/// need — load and register it (see [`load_feature_shaders`]).
fn load_core_3d_shader(world: &mut moonfield_app::prelude::World) {
    load_feature_shaders(
        world,
        &[(
            crate::core_3d::pass::CORE_3D_SHADER,
            "core_3d.slang",
            crate::core_3d::pass::core_3d_shader,
        )],
    );
}

/// Registers the renderer's ECS surface: the `Assets` stores entities
/// reference through `MeshRenderer` / `SplatCloudHandle`, the shader-asset
/// pipeline (`PipelineShaders` requests → extraction → `PreparedShaders`),
/// and the per-frame extraction of those components into the render world.
///
/// The plugin initializes its stores only when absent (a preconfigured store
/// survives composition) and declares its own shader needs at startup, so a
/// minimal app renders meshes without the editor.
pub struct RenderFeaturePlugin;

impl Plugin for RenderFeaturePlugin {
    fn name(&self) -> &str {
        "moonfield_render_feature::RenderFeaturePlugin"
    }

    fn build(&self, app: &mut App) {
        app.init_resource::<moonfield_asset::Assets<Mesh>>();
        #[cfg(feature = "splat")]
        app.init_resource::<moonfield_asset::Assets<SplatCloud>>();
        app.init_resource::<moonfield_asset::Assets<moonfield_shader::Shader>>();
        app.init_resource::<PipelineShaders>();
        app.add_systems(Startup, load_core_3d_shader);

        app.add_render_systems(
            ExtractSchedule,
            (
                extract_mesh_assets,
                extract_shader_assets,
                extract_with_transform::<MeshRenderer>,
            ),
        );
        app.render_world_mut().init_resource::<PreparedGpuMeshes>();
        app.render_world_mut().init_resource::<PreparedShaders>();
        app.render_world_mut()
            .init_resource::<DrawFunctions<Opaque3d>>();
        app.render_world_mut()
            .get_resource_mut::<DrawFunctions<Opaque3d>>()
            .expect("DrawFunctions<Opaque3d> was just ensured")
            .register::<DrawMesh>();

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
            debug_scene_log
                .after(&queue_opaque_3d)
                .in_set::<render_sets::Queue>(),
        );
        app.add_render_systems(
            Render,
            (
                // The pooled offscreen targets must exist before render-core
                // resolves the per-view attachment components.
                crate::core_3d::pass::prepare_view_targets.before(&prepare_view_attachments),
                crate::core_3d::pass::prepare_core_3d_pipeline,
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

    /// Composition check: the feature's systems anchor on the `Render`
    /// schedule's set chain, which `moonfield_render_core::RenderPlugin`
    /// registers. Missing anchors mean the plugin was composed without the
    /// render core — fail with the remedy rather than silently misordering.
    fn finish(&self, app: &mut App) {
        let schedules = app
            .render_world()
            .get_resource::<moonfield_ecs::Schedules>();
        let render = schedules
            .as_deref()
            .and_then(|schedules| schedules.get(&std::any::TypeId::of::<Render>()));
        let missing: Vec<&'static str> = [
            render.is_some_and(|s| s.has_set::<render_sets::PrepareAssets>()),
            render.is_some_and(|s| s.has_set::<render_sets::Queue>()),
            render.is_some_and(|s| s.has_set::<render_sets::PrepareViews>()),
            render.is_some_and(|s| s.has_set::<render_sets::CameraDriver>()),
        ]
        .into_iter()
        .zip(["PrepareAssets", "Queue", "PrepareViews", "CameraDriver"])
        .filter_map(|(present, name)| (!present).then_some(name))
        .collect();
        assert!(
            missing.is_empty(),
            "RenderFeaturePlugin requires moonfield_render_core::RenderPlugin: \
             the Render schedule is missing the {} anchor(s). \
             Add RenderPlugin before RenderFeaturePlugin.",
            missing.join(", ")
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
    fn render_feature_plugin_preserves_preconfigured_stores() {
        let mut app = App::new();
        // The caller's store, with content, exists before the plugin builds.
        let mesh = {
            let mut meshes = moonfield_asset::Assets::<Mesh>::default();
            let mesh = meshes.add(Mesh::new(vec![[0.0; 3]], vec![0], None));
            app.insert_resource(meshes);
            mesh
        };
        app.add_plugin(RenderFeaturePlugin);
        let meshes = app
            .world()
            .get_resource::<moonfield_asset::Assets<Mesh>>()
            .unwrap();
        assert_eq!(meshes.len(), 1);
        assert!(meshes.get(&mesh).is_some());
    }

    #[test]
    fn render_feature_plugin_loads_its_own_shader_at_startup() {
        // No editor: the plugin declares and registers the core 3D shader
        // itself at startup.
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        app.render();
        let requests = app
            .world()
            .get_resource::<PipelineShaders>()
            .expect("PipelineShaders");
        assert!(
            requests.get(crate::core_3d::pass::CORE_3D_SHADER).is_some(),
            "core 3d shader request registered"
        );
    }

    #[test]
    #[should_panic(expected = "requires moonfield_render_core::RenderPlugin")]
    fn render_feature_plugin_finish_requires_render_plugin_anchors() {
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        // Composition is complete without RenderPlugin: finish must fail.
        Plugin::finish(&RenderFeaturePlugin, &mut app);
    }

    #[test]
    fn test_render_feature_startup_preserves_application_shader_request() {
        let mut app = App::new();
        app.add_plugin(RenderFeaturePlugin);
        let shader = app
            .world()
            .get_resource_mut::<moonfield_asset::Assets<moonfield_shader::Shader>>()
            .unwrap()
            .add(moonfield_shader::Shader::new(
                "custom".into(),
                "application.slang".into(),
            ));
        let mut request = crate::core_3d::pass::core_3d_shader(shader);
        request.reflect_entry = "application_entry";
        app.world()
            .get_resource_mut::<PipelineShaders>()
            .unwrap()
            .push(request);
        app.render();
        let requests = app.world().get_resource::<PipelineShaders>().unwrap();
        let actual = requests.get(crate::core_3d::pass::CORE_3D_SHADER).unwrap();
        assert_eq!(actual.shader, shader);
        assert_eq!(actual.reflect_entry, "application_entry");
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
                moonfield_rhi::testing::skip_gpu(format!("no Vulkan device available ({error})"));
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

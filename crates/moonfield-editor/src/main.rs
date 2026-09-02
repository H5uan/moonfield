//! The moonfield editor binary — the workspace's only executable entry point.
//!
//! Wires `RenderPlugin` (shared `RenderDevice`) + `WinitPlugin` (continuous
//! update mode for redraws) + `HierarchyPlugin` (transform propagation) +
//! `EditorPlugin`, loads the repository's teapot mesh into the scene,
//! and runs the app. Set `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` to exit
//! after N rendered frames — used by automated startup/shutdown smoke tests
//! on machines with a display and a Vulkan driver.
//!
//! ```sh
//! MOONFIELD_EDITOR_AUTO_CLOSE=5 cargo run
//! ```

use moonfield_app::App;
use moonfield_app::prelude::{HierarchyPlugin, Name, Startup, World};
use moonfield_camera::{Camera, PrimaryCamera};
use moonfield_editor::{EditorPlugin, load_asset};
use moonfield_log::LogPlugin;
use moonfield_math::{Transform, Vec3};
use moonfield_render_core::RenderPlugin;
use moonfield_render_feature::RenderFeaturePlugin;
use moonfield_winit::{WinitPlugin, WinitSettings};
use std::path::PathBuf;

fn default_mesh_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/models/teapot.glb")
}

fn main() -> std::process::ExitCode {
    let mut app = App::new();
    app.add_plugin(LogPlugin::default());
    app.add_plugin(RenderPlugin);
    app.add_plugin(RenderFeaturePlugin);
    app.add_plugin(HierarchyPlugin);
    app.add_plugin(WinitPlugin::default().with_settings(WinitSettings::continuous()));
    app.add_plugin(EditorPlugin);
    app.add_systems(Startup, spawn_default_scene);
    app.run().code
}

/// The default scene: a primary camera and the repository-managed teapot mesh.
fn spawn_default_scene(world: &mut World) {
    world.spawn((
        Name::new("Main Camera"),
        Camera::default(),
        PrimaryCamera,
        Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
    ));

    let path = default_mesh_path();
    load_asset(world, &path)
        .unwrap_or_else(|error| panic!("failed to load default mesh {}: {error}", path.display()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_asset::{AssetServer, Assets};
    use moonfield_render_feature::mesh::{Mesh, MeshRenderer};

    #[test]
    fn test_default_scene_loads_repository_teapot() {
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(moonfield_editor::editor_asset_server());

        spawn_default_scene(&mut world);

        assert!(default_mesh_path().is_file());
        assert_eq!(world.query::<&PrimaryCamera>().count(), 1);
        let (_, renderer) = world
            .query::<&MeshRenderer>()
            .next()
            .expect("teapot entity");
        let meshes = world.get_resource::<Assets<Mesh>>().unwrap();
        let mesh = meshes.get(&renderer.mesh.0).expect("teapot mesh asset");
        assert!(mesh.positions().len() > 1_000_000);
        assert!(!mesh.indices().is_empty());
        drop(meshes);
        assert!(world.contains_resource::<AssetServer>());
    }
}

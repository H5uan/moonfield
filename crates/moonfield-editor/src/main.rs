//! The moonfield editor binary — the workspace's only executable entry point.
//!
//! Wires `RenderPlugin` (shared `RenderDevice`) + `WinitPlugin` (continuous
//! update mode for redraws) + `HierarchyPlugin` (transform propagation) +
//! `EditorPlugin`, spawns a small demo scene (camera + parent/child cubes),
//! and runs the app. Set `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` to exit
//! after N rendered frames — used by automated startup/shutdown smoke tests
//! on machines with a display and a Vulkan driver.
//!
//! ```sh
//! MOONFIELD_EDITOR_AUTO_CLOSE=5 cargo run
//! ```

use moonfield_app::prelude::{ChildOf, HierarchyPlugin, Name, Startup, World};
use moonfield_app::App;
use moonfield_editor::EditorPlugin;
use moonfield_log::LogPlugin;
use moonfield_math::{Transform, Vec3};
use moonfield_render::{Camera, MeshRenderer, PrimaryCamera, RenderPlugin};
use moonfield_winit::{WinitPlugin, WinitSettings};

fn main() {
    let mut app = App::new();
    app.add_plugin(LogPlugin::default());
    app.add_plugin(RenderPlugin);
    app.add_plugin(HierarchyPlugin);
    app.add_plugin(WinitPlugin::default().with_settings(WinitSettings::continuous()));
    app.add_plugin(EditorPlugin);
    app.add_systems(Startup, spawn_demo_scene);
    app.run();
}

/// The demo scene: a primary camera and a parent cube carrying a smaller,
/// offset child cube (exercises hierarchy propagation in the viewport).
fn spawn_demo_scene(world: &mut World) {
    world.spawn((
        Name::new("Main Camera"),
        Camera::default(),
        PrimaryCamera,
        Transform::from_xyz(0.0, 2.5, 6.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
    ));

    let parent = world.spawn((
        Name::new("Parent Cube"),
        MeshRenderer::colored([0.35, 0.5, 0.9, 1.0]),
        Transform::from_xyz(-0.75, 0.0, 0.0),
    ));
    world.spawn((
        Name::new("Child Cube"),
        MeshRenderer::colored([0.9, 0.55, 0.25, 1.0]),
        Transform {
            translation: Vec3::new(1.5, 1.0, 0.0),
            scale: Vec3::splat(0.5),
            ..Transform::IDENTITY
        },
        ChildOf(parent),
    ));
}

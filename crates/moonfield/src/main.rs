//! Moonfield sample application entry point.

use moonfield_app::prelude::World;
use moonfield_app::App;
use moonfield_log::info;
use moonfield_render::RenderPlugin;
use moonfield_script::ScriptPlugin;
use moonfield_winit::WinitPlugin;

fn main() {
    let mut app = App::new();

    app.add_plugin(moonfield_log::LogPlugin::default());

    // ECS 资源
    app.insert_resource(Time::default());

    // ECS 系统
    app.add_startup_system(|_world: &mut World| {
        info!("ECS startup!");
    });
    app.add_systems(print_fps);

    app.add_plugin(ScriptPlugin);
    app.add_plugin(RenderPlugin);
    app.add_plugin(WinitPlugin::default());

    app.run();
}

fn print_fps(world: &mut World) {
    if let Some(time) = world.get_resource::<Time>() {
        info!("FPS delta: {}", time.delta);
    }
}

#[derive(Default)]
struct Time {
    delta: f32,
}

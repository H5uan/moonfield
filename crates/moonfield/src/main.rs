//! Moonfield sample application entry point.

use moonfield_app::prelude::Res;
use moonfield_app::{App, Startup, TimePlugin, Update};
use moonfield_log::info;
use moonfield_render::RenderPlugin;
use moonfield_time::Time;
use moonfield_winit::WinitPlugin;

fn main() {
    let mut app = App::new();

    app.add_plugin(moonfield_log::LogPlugin::default());
    app.add_plugin(TimePlugin);

    // ECS 系统
    app.add_systems(Startup, || {
        info!("ECS startup!");
    });
    app.add_systems(Update, print_fps);

    app.add_plugin(RenderPlugin);
    app.add_plugin(WinitPlugin::default());

    app.run();
}

fn print_fps(time: Res<Time>) {
    info!("FPS delta: {}", time.delta_secs());
}

//! Moonfield sample application entry point.

// Script host API bindings (composition root). The script system is
// temporarily unplugged from the app below — no `ScriptPlugin` is added, so
// nothing calls into this module at runtime. It stays compiled so the
// `moonfield.d.ts` sync test keeps guarding the bindings; re-enable by
// wiring `ScriptPlugin` back into `main`.
#[allow(dead_code)]
mod script_api;

use moonfield_app::prelude::World;
use moonfield_app::App;
use moonfield_log::info;
use moonfield_render::RenderPlugin;
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

    // Script system temporarily unplugged: no `ScriptPlugin` is added, so no
    // script runtime is created and `scripts/record_frame.ts` is not loaded.
    // The window control/state/requests handles revert to the plugin's own
    // defaults. Re-enable by restoring the `ScriptPlugin::new(...)` wiring
    // here (see git history).

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

//! Moonfield sample application entry point.

mod script_api;

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

    // 脚本系统：host API 在 script_api 模块组装（组合根模式）
    let input = moonfield_script::new_shared_input();
    let time = moonfield_script::new_shared_time();
    let window_control = moonfield_window::WindowControl::default();
    let window_state = moonfield_window::new_shared_window();
    let window_requests = moonfield_window::WindowRequests::default();
    let plugin = ScriptPlugin::new(script_api::build_script_api(
        &input,
        &time,
        &window_control,
        &window_state,
        &window_requests,
    ))
    .with_input_state(input)
    .with_time_state(time);
    let plugin = plugin.with_configure(script_api::configure_runtime);
    app.add_plugin(plugin);

    app.add_plugin(RenderPlugin);
    app.add_plugin(
        WinitPlugin::default()
            .with_window_control(window_control)
            .with_window_state(window_state)
            .with_window_requests(window_requests),
    );

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

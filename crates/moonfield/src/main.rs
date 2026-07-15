//! Moonfield sample application entry point.

use moonfield_app::App;
use moonfield_lunaris::LunarisPlugin;
use moonfield_script::ScriptPlugin;
use moonfield_winit::WinitPlugin;

fn main() {
    let mut app = App::new();
    app.add_plugin(ScriptPlugin);
    app.add_plugin(LunarisPlugin);
    app.add_plugin(WinitPlugin::default());

    app.run();
}

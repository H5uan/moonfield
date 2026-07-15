//! Moonfield sample application entry point.

use moonfield_base::info;
use moonfield_core::App;
use moonfield_lunaris::LunarisPlugin;
use moonfield_script::ScriptPlugin;

fn main() {
    let mut app = App::new();
    app.add_plugin(ScriptPlugin);
    app.add_plugin(LunarisPlugin);

    let exit_code = app.run(|app| {
        info!("Hello from modular moonfield!");
        app.run_updates();
        0
    });

    std::process::exit(exit_code);
}

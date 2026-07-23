//! Moonfield editor entry point.
//!
//! The editor is a separate binary composing the engine's plugin crates: it
//! installs its own egui-based event loop instead of `WinitPlugin` and owns
//! the windowed Vulkan renderer. Scripting (play mode) is not wired in yet.

mod runner;
mod ui;
mod viewport;

use moonfield_app::App;

fn main() {
    let mut app = App::new();

    app.add_plugin(moonfield_log::LogPlugin::default());
    app.add_plugin(runner::EditorPlugin);

    app.run();
}

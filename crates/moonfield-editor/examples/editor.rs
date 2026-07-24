//! Minimal editor smoke target.
//!
//! Wires `WinitPlugin` (Poll mode for continuous redraw) + `EditorPlugin`
//! and runs the app. Set `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` to exit
//! after N rendered frames — used by scripted startup/shutdown smoke tests
//! on machines with a display and a Vulkan driver.
//!
//! ```sh
//! MOONFIELD_EDITOR_AUTO_CLOSE=5 cargo run --example editor -p moonfield-editor
//! ```

use moonfield_app::App;
use moonfield_editor::EditorPlugin;
use moonfield_log::LogPlugin;
use moonfield_winit::{WaitMode, WinitPlugin};

fn main() {
    let mut app = App::new();
    app.add_plugin(LogPlugin::default());
    app.add_plugin(WinitPlugin::default().with_wait_mode(WaitMode::Poll));
    app.add_plugin(EditorPlugin);
    app.run();
}

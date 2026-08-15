//! Minimal editor smoke target.
//!
//! Wires `WinitPlugin` (continuous update mode for redraws) + `EditorPlugin`
//! and runs the app. Set `MOONFIELD_EDITOR_AUTO_CLOSE=<frames>` to exit
//! after N rendered frames — used by automated startup/shutdown smoke tests
//! on machines with a display and a Vulkan driver.
//!
//! ```sh
//! MOONFIELD_EDITOR_AUTO_CLOSE=5 cargo run --example editor -p moonfield-editor
//! ```

use moonfield_app::App;
use moonfield_editor::EditorPlugin;
use moonfield_log::LogPlugin;
use moonfield_winit::{WinitPlugin, WinitSettings};

fn main() {
    let mut app = App::new();
    app.add_plugin(LogPlugin::default());
    app.add_plugin(WinitPlugin::default().with_settings(WinitSettings::continuous()));
    app.add_plugin(EditorPlugin);
    app.run();
}

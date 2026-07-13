//! Runtime application plugin.
//!
//! Provides a `RuntimePlugin` that registers the core runtime services and
//! lifecycle systems with the application.

pub mod script;

use moonfield_base::info;
use moonfield_core::{App, Plugin, Resources};
use script::{QuickJsRuntime, ScriptApi, ScriptRuntime};
use std::path::Path;

/// Runtime plugin.
pub struct RuntimePlugin;

impl Plugin for RuntimePlugin {
    fn name(&self) -> &str {
        "Runtime"
    }

    fn build(&self, app: &mut App) {
        app.add_startup_system(|_res: &mut Resources| {
            info!("Runtime startup system");
            if let Err(e) = run_default_script() {
                info!("Failed to run default script: {}", e);
            }
        });
        app.add_shutdown_system(|_res: &mut Resources| {
            info!("Runtime shutdown system");
        });
    }
}

/// Run the default script entry point.
///
/// Loads `scripts/record_frame.js` (or `.ts`), registers host APIs, and calls
/// the top-level `main()` function if it exists.
pub fn run_default_script() -> crate::script::Result<()> {
    let script_dir = Path::new("scripts");
    let js_path = script_dir.join("record_frame.js");
    let ts_path = script_dir.join("record_frame.ts");
    let script_path = if js_path.exists() {
        js_path
    } else {
        ts_path
    };

    let source = script::load_script(&script_path)?;
    let mut runtime = QuickJsRuntime::new(ScriptApi::default())?;
    runtime.load(script_path.to_string_lossy().as_ref(), &source)?;
    let _ = runtime.call("main");
    Ok(())
}

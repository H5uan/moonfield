//! Scripting runtime plugin and module system.
//!
//! Provides a `ScriptPlugin` that registers the script runtime with the
//! application. The crate is exclusively focused on the script system:
//! TypeScript/JavaScript execution via V8 or QuickJS, module loading,
//! hot-reload, and host API bindings.

pub mod script;

use moonfield_app::prelude::World;
use moonfield_app::{App, Plugin};
use moonfield_log::info;
#[cfg(feature = "v8-backend")]
use script::{ScriptApi, ScriptRuntime, V8Runtime as Runtime};

#[cfg(feature = "quickjs-backend")]
use script::{QuickJsRuntime as Runtime, ScriptApi, ScriptRuntime};
use std::path::Path;

/// Script system plugin.
///
/// Runs the default script (`scripts/record_frame.ts` or `.js`) on startup.
pub struct ScriptPlugin;

impl Plugin for ScriptPlugin {
    fn name(&self) -> &str {
        "Script"
    }

    fn build(&self, app: &mut App) {
        app.add_startup_system(|_world: &mut World| {
            info!("Script plugin startup");
            if let Err(e) = run_default_script() {
                info!("Failed to run default script: {}", e);
            }
        });
        app.add_shutdown_system(|_world: &mut World| {
            info!("Script plugin shutdown");
        });
    }
}

/// Run the default script entry point.
///
/// Loads `scripts/record_frame.js` (or `.ts`), registers host APIs, and calls
/// the top-level `main()` function if it exists.
///
/// TypeScript is loaded as pre-compiled JavaScript (from `target/scripts/`
/// or alongside the `.ts` file). The V8 backend requires pre-compiled JS;
/// the QuickJS backend can fall back to swc-based transpilation at runtime.
pub fn run_default_script() -> script::Result<()> {
    let script_dir = Path::new("scripts");
    let js_path = script_dir.join("record_frame.js");
    let ts_path = script_dir.join("record_frame.ts");
    let script_path = if js_path.exists() { js_path } else { ts_path };

    let source = script::load_script(&script_path)?;
    let mut runtime = Runtime::new(ScriptApi::default())?;
    runtime.load(script_path.to_string_lossy().as_ref(), &source)?;
    // Warm up JIT so the first frame-loop iteration runs compiled code.
    let _ = runtime.warmup("main");
    let _ = runtime.call("main");
    Ok(())
}

/// Run a script module using the ESModule module system.
///
/// Resolves all transitive dependencies, then loads and evaluates the module
/// graph via V8's native ESModule API.
///
/// # Example
///
/// ```ts
/// // scripts/main.ts
/// import { record_frame } from "./record_frame.js";
/// export function main() { record_frame(); }
/// ```
#[cfg(feature = "v8-backend")]
pub fn run_script_module(entry: &str) -> crate::script::Result<()> {
    use script::ModuleRegistry;

    let entry_path = Path::new(entry);
    let source = script::load_script(entry_path)?;

    let mut registry = ModuleRegistry::new();
    let canonical_name = registry.register(entry, source);

    // Resolve and register all transitive dependencies.
    registry.resolve_dependencies(&canonical_name)
        .map_err(|e| script::ScriptError::Execution(e))?;

    let mut runtime = Runtime::new(ScriptApi::default())?;

    // Load and evaluate the module graph, then call main().
    runtime.load_module_graph(&registry, &canonical_name)
}

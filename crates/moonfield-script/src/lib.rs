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
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Script system plugin.
///
/// Runs the default script (`scripts/record_frame.ts` or `.js`) on startup.
/// The runtime is kept alive across frames so that `gc_step()` can be called
/// each frame for incremental GC control.
pub struct ScriptPlugin;

impl Plugin for ScriptPlugin {
    fn name(&self) -> &str {
        "Script"
    }

    fn build(&self, app: &mut App) {
        // V8Runtime is !Send + !Sync, so it can't be a World resource.
        // Use Rc<RefCell<Option<Runtime>>> — the update system only
        // requires FnMut + 'static (no Send).
        let runtime_slot: Rc<RefCell<Option<Runtime>>> = Rc::new(RefCell::new(None));

        let slot = runtime_slot.clone();
        app.add_startup_system(move |_world: &mut World| {
            info!("Script plugin startup");
            match Runtime::new(ScriptApi::default()) {
                Ok(mut runtime) => {
                    let script_dir = Path::new("scripts");
                    let js_path = script_dir.join("record_frame.js");
                    let ts_path = script_dir.join("record_frame.ts");
                    let script_path = if js_path.exists() { js_path } else { ts_path };
                    match script::load_script(&script_path) {
                        Ok(source) => {
                            let _ = runtime.load(script_path.to_string_lossy().as_ref(), &source);
                            let _ = runtime.warmup("main");
                            let _ = runtime.call("main");
                        }
                        Err(e) => info!("Failed to load script: {}", e),
                    }
                    *slot.borrow_mut() = Some(runtime);
                }
                Err(e) => info!("Failed to create script runtime: {}", e),
            }
        });

        let slot = runtime_slot.clone();
        app.add_update_system(move |_world: &mut World| {
            if let Some(rt) = slot.borrow_mut().as_mut() {
                rt.gc_step();
            }
            true
        });

        let slot = runtime_slot;
        app.add_shutdown_system(move |_world: &mut World| {
            info!("Script plugin shutdown");
            *slot.borrow_mut() = None;
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
    registry
        .resolve_dependencies(&canonical_name)
        .map_err(|e| script::ScriptError::Execution(e))?;

    let mut runtime = Runtime::new(ScriptApi::default())?;

    // Load and evaluate the module graph, then call main().
    runtime.load_module_graph(&registry, &canonical_name)
}

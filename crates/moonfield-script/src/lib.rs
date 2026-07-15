//! Scripting runtime plugin and module system.
//!
//! Provides a `ScriptPlugin` that registers the script runtime with the
//! application. The crate is exclusively focused on the script system:
//! TypeScript/JavaScript execution via V8 or QuickJS, module loading,
//! hot-reload, and host API bindings.

pub mod script;

use moonfield_app::{App, Plugin, Resources};
use moonfield_base::info;
#[cfg(feature = "v8-backend")]
use script::{ScriptApi, ScriptRuntime, V8Runtime as Runtime};

#[cfg(feature = "quickjs-backend")]
use script::{QuickJsRuntime as Runtime, ScriptApi, ScriptRuntime};
use std::path::{Path, PathBuf};

/// Script system plugin.
///
/// Runs the default script (`scripts/record_frame.ts` or `.js`) on startup.
pub struct ScriptPlugin;

impl Plugin for ScriptPlugin {
    fn name(&self) -> &str {
        "Script"
    }

    fn build(&self, app: &mut App) {
        app.add_startup_system(|_res: &mut Resources| {
            info!("Script plugin startup");
            if let Err(e) = run_default_script() {
                info!("Failed to run default script: {}", e);
            }
        });
        app.add_shutdown_system(|_res: &mut Resources| {
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
/// or alongside the `.ts` file). The V8 backend also supports native TS
/// type stripping via `--strip-types`, so raw `.ts` source can be loaded
/// directly without preprocessing.
pub fn run_default_script() -> script::Result<()> {
    let script_dir = Path::new("scripts");
    let js_path = script_dir.join("record_frame.js");
    let ts_path = script_dir.join("record_frame.ts");
    let script_path = if js_path.exists() { js_path } else { ts_path };

    let source = script::load_script(&script_path)?;
    let mut runtime = Runtime::new(ScriptApi::default())?;
    runtime.load(script_path.to_string_lossy().as_ref(), &source)?;
    let _ = runtime.call("main");
    Ok(())
}

/// Run a script module using the CommonJS-based module system.
///
/// Each module's `import`/`export` is transformed to `__require`/`exports`
/// globals, then evaluated in topological dependency order.
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
    resolve_dependencies(&mut registry, &canonical_name)?;

    let mut runtime = Runtime::new(ScriptApi::default())?;

    // Load and evaluate the module graph, then call main().
    runtime.load_module_graph(&registry, &canonical_name)
}

/// Recursively resolve and register all dependencies of a module.
///
/// Uses the full resolution chain: relative paths, bare specifiers,
/// `node_modules` lookup, `package.json` main field, and `index.js` fallback.
#[cfg(feature = "v8-backend")]
fn resolve_dependencies(registry: &mut script::ModuleRegistry, name: &str) -> crate::script::Result<()> {
    let deps: Vec<String> = {
        let info = registry
            .get(name)
            .ok_or_else(|| script::ScriptError::Execution(format!("module '{}' not found", name)))?;
        info.imports.clone()
    };

    let search_dirs = vec![
        PathBuf::from("."),
        PathBuf::from("scripts"),
        PathBuf::from("target/scripts"),
    ];

    for dep in &deps {
        // Try the in-registry resolve first (fast path).
        let resolved = registry.resolve(dep, name);

        if let Some(resolved) = resolved {
            if !registry.contains(&resolved) {
                // Try to find the file on disk using the resolved name.
                let dep_path = Path::new(&resolved);
                let candidates = [
                    dep_path.to_path_buf(),
                    dep_path.with_extension("js"),
                    dep_path.with_extension("ts"),
                    Path::new("scripts").join(&resolved).with_extension("js"),
                    Path::new("scripts").join(&resolved).with_extension("ts"),
                    Path::new("target/scripts").join(&resolved).with_extension("js"),
                ];

                let mut loaded = false;
                for candidate in &candidates {
                    if candidate.exists() {
                        let source = script::load_script(candidate)?;
                        registry.register(&resolved, source);
                        loaded = true;
                        break;
                    }
                }

                if !loaded {
                    return Err(script::ScriptError::Execution(format!(
                        "module '{}' not found on disk",
                        resolved
                    )));
                }

                resolve_dependencies(registry, &resolved)?;
            }
        } else {
            // Full resolution chain for bare specifiers and node_modules.
            let (canonical, path) = registry
                .resolve_full(dep, name, &search_dirs)
                .ok_or_else(|| script::ScriptError::Execution(format!(
                    "cannot resolve '{}' from '{}'",
                    dep, name
                )))?;

            if !registry.contains(&canonical) {
                let source = script::load_script(&path)?;
                registry.register(&canonical, source);
                resolve_dependencies(registry, &canonical)?;
            }
        }
    }

    Ok(())
}

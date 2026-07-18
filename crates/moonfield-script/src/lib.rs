//! Scripting runtime plugin and module system.
//!
//! Provides a `ScriptPlugin` that registers the script runtime with the
//! application. The crate is exclusively focused on the script system:
//! TypeScript/JavaScript execution via V8 or QuickJS, module loading,
//! hot-reload, and host API bindings.
//!
//! Host functions are provided by the embedding application (see
//! [`ScriptApi`]); this crate deliberately has no engine-layer dependencies.

pub mod script;

pub use moonfield_script_macros::script_function;

use moonfield_app::prelude::World;
use moonfield_app::{App, Plugin};
use moonfield_log::{error, info, warn};

#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
pub use script::V8Runtime as Runtime;

#[cfg(feature = "quickjs-backend")]
pub use script::QuickJsRuntime as Runtime;

use script::{HostValue, HotReloader, ScriptApi, ScriptRuntime};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// Runtime configuration hook used by [`ScriptPlugin::with_configure`],
/// invoked right after the runtime is created.
type ConfigureFn = Arc<dyn Fn(&mut Runtime) + Send + Sync>;

/// Script system plugin.
///
/// Loads an entry script (default `scripts/record_frame.js`, falling back to
/// `.ts`) as an ESModule graph on startup, watches the entry's directory for
/// changes and hot-reloads affected modules, and drives optional script-side
/// lifecycle hooks:
///
/// - `main()` — called once after the module graph is first evaluated.
/// - `on_update(dt)` — called every frame with the frame delta in seconds.
/// - `on_shutdown()` — called once when the app shuts down.
///
/// Missing hooks are skipped silently. Script errors are logged and do not
/// take down the app; a failed hot reload keeps the previously evaluated
/// code running.
///
/// The runtime is kept alive across frames in an `Rc<RefCell<_>>` (the
/// runtimes are `!Send`, so they cannot be World resources) so that
/// incremental GC and hot reload can run each frame.
pub struct ScriptPlugin {
    /// Host functions exposed to scripts, built by the embedding application.
    api: ScriptApi,
    /// Entry script path; defaults to `scripts/record_frame.js` / `.ts`.
    entry: Option<PathBuf>,
    /// Extra runtime configuration hook (e.g. registering V8 direct
    /// functions). Runs right after the runtime is created, before the entry
    /// module is loaded.
    configure: Option<ConfigureFn>,
    /// Max JS heap size in bytes; `None` uses the backend default (128 MB).
    memory_limit: Option<usize>,
}

impl Default for ScriptPlugin {
    fn default() -> Self {
        Self::new(ScriptApi::default())
    }
}

impl ScriptPlugin {
    /// Create a plugin that exposes `api` to scripts.
    pub fn new(api: ScriptApi) -> Self {
        Self {
            api,
            entry: None,
            configure: None,
            memory_limit: None,
        }
    }

    /// Override the entry script path (default: `scripts/record_frame.js`
    /// falling back to `scripts/record_frame.ts`).
    pub fn with_entry(mut self, entry: impl Into<PathBuf>) -> Self {
        self.entry = Some(entry.into());
        self
    }

    /// Cap the JS engine's heap at `bytes` (default: 128 MB).
    pub fn with_memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit = Some(bytes);
        self
    }

    /// Add a hook that runs right after the script runtime is created, e.g.
    /// to register backend-specific fast-path host functions:
    ///
    /// ```ignore
    /// ScriptPlugin::new(api).with_configure(|rt| {
    ///     rt.register_direct("record_frame", direct_record_frame);
    /// });
    /// ```
    pub fn with_configure(
        mut self,
        configure: impl Fn(&mut Runtime) + Send + Sync + 'static,
    ) -> Self {
        self.configure = Some(Arc::new(configure));
        self
    }
}

/// Per-app script runtime state shared by the plugin's systems.
struct ScriptState {
    runtime: Runtime,
    hot_reloader: Option<HotReloader>,
    /// Wall-clock of the previous update, used to compute `on_update(dt)`.
    last_frame: Instant,
}

impl Plugin for ScriptPlugin {
    fn name(&self) -> &str {
        "Script"
    }

    fn build(&self, app: &mut App) {
        // Runtimes are !Send + !Sync, so they can't be World resources.
        // Use Rc<RefCell<Option<ScriptState>>> — the systems only require
        // FnMut + 'static (no Send).
        let state_slot: Rc<RefCell<Option<ScriptState>>> = Rc::new(RefCell::new(None));

        let api = self.api.clone();
        let entry = self.entry.clone();
        let configure = self.configure.clone();
        let memory_limit = self.memory_limit;
        let slot = state_slot.clone();
        app.add_startup_system(move |_world: &mut World| {
            info!("Script plugin startup");
            let mut runtime = match Runtime::new_with_memory_limit(
                api,
                memory_limit.unwrap_or(script::DEFAULT_MAX_HEAP_BYTES),
            ) {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to create script runtime: {}", e);
                    return;
                }
            };
            if let Some(configure) = &configure {
                configure(&mut runtime);
            }
            let entry_path = entry.unwrap_or_else(default_script_path);
            if let Err(e) = load_module_entry(&mut runtime, &entry_path) {
                error!("Failed to load script '{}': {}", entry_path.display(), e);
                return;
            }
            // Watch the entry's directory for hot reload. `.ts` sources are
            // transpiled in-process on (re)load, so edits take effect
            // directly; a pre-compiled `.js` (e.g. from `tsc`, with source
            // maps) is preferred when present.
            let watch_dir = entry_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf();
            let hot_reloader = match HotReloader::new(&watch_dir) {
                Ok(reloader) => Some(reloader),
                Err(e) => {
                    warn!(
                        "Hot reload disabled (cannot watch '{}'): {}",
                        watch_dir.display(),
                        e
                    );
                    None
                }
            };
            *slot.borrow_mut() = Some(ScriptState {
                runtime,
                hot_reloader,
                last_frame: Instant::now(),
            });
        });

        let slot = state_slot.clone();
        app.add_update_system(move |_world: &mut World| {
            let mut slot = slot.borrow_mut();
            let Some(state) = slot.as_mut() else {
                return true;
            };
            state.runtime.gc_step();
            if let Some(reloader) = state.hot_reloader.as_mut() {
                if let Err(e) = reloader.poll(&mut state.runtime) {
                    error!("Hot reload failed: {}", e);
                }
            }
            let now = Instant::now();
            let dt = now.duration_since(state.last_frame).as_secs_f64();
            state.last_frame = now;
            if state.runtime.has_function("on_update") {
                if let Err(e) = state
                    .runtime
                    .call_module_export_unit("on_update", &[HostValue::Number(dt)])
                {
                    error!("script on_update failed: {}", e);
                }
            }
            true
        });

        let slot = state_slot;
        app.add_shutdown_system(move |_world: &mut World| {
            info!("Script plugin shutdown");
            if let Some(state) = slot.borrow_mut().as_mut() {
                if state.runtime.has_function("on_shutdown") {
                    if let Err(e) = state.runtime.call_module_export_unit("on_shutdown", &[]) {
                        error!("script on_shutdown failed: {}", e);
                    }
                }
            }
            *slot.borrow_mut() = None;
        });
    }
}

/// Resolve the default entry script: `scripts/record_frame.js` if present,
/// else `scripts/record_frame.ts`.
fn default_script_path() -> PathBuf {
    let script_dir = Path::new("scripts");
    let js_path = script_dir.join("record_frame.js");
    if js_path.exists() {
        js_path
    } else {
        script_dir.join("record_frame.ts")
    }
}

/// Load an entry script as an ESModule graph: register it, resolve all
/// transitive dependencies from disk, then compile/instantiate/evaluate the
/// graph and call the entry's `main()` export if present.
///
/// Returns the canonical entry name.
///
/// # Example
///
/// ```ts
/// // scripts/main.ts
/// import { record_frame } from "./record_frame.js";
/// export function main() { record_frame(); }
/// ```
pub fn load_module_entry(runtime: &mut Runtime, entry_path: &Path) -> script::Result<String> {
    use script::ModuleRegistry;

    let entry_str = entry_path.to_string_lossy().replace('\\', "/");
    let source = script::load_script(entry_path)?;

    let mut registry = ModuleRegistry::new();
    let canonical_name = registry.register(&entry_str, source);

    // Resolve and register all transitive dependencies.
    registry
        .resolve_dependencies(&canonical_name)
        .map_err(script::ScriptError::Execution)?;

    // Load and evaluate the module graph, then call main().
    runtime.load_module_graph(Rc::new(registry), &canonical_name)?;
    Ok(canonical_name)
}

/// Run a script module entry point to completion using the module system.
///
/// Convenience wrapper around [`load_module_entry`] for one-shot usage.
pub fn run_script_module(entry: &str, api: ScriptApi) -> script::Result<()> {
    let mut runtime = Runtime::new(api)?;
    load_module_entry(&mut runtime, Path::new(entry))?;
    Ok(())
}

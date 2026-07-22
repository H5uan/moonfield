//! Scripting runtime plugin and module system.
//!
//! Provides a `ScriptPlugin` that registers the script runtime with the
//! application. The crate is exclusively focused on the script system:
//! TypeScript/JavaScript execution via V8 or QuickJS, module loading,
//! hot-reload, and host API bindings.
//!
//! Host functions are provided by the embedding application (see
//! [`ScriptApi`]); this crate deliberately has no engine-layer dependencies.

pub mod input;
pub mod script;
pub mod time;
pub mod window;

pub use input::{new_shared_input, register_input_api, ScriptInputState, SharedInputState};
pub use moonfield_script_macros::script_function;
pub use time::{new_shared_time, register_time_api, ScriptTimeState, SharedTimeState};
pub use window::register_window_api;

use moonfield_app::prelude::World;
use moonfield_app::{App, Plugin};
use moonfield_log::{error, info, warn};

#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
pub use script::V8Runtime as Runtime;

#[cfg(feature = "quickjs-backend")]
pub use script::QuickJsRuntime as Runtime;

use script::{HostValue, HotReloadHandler, HotReloader, ScriptApi, ScriptRuntime};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Runtime configuration hook used by [`ScriptPlugin::with_configure`],
/// invoked right after the runtime is created.
type ConfigureFn = Arc<dyn Fn(&mut Runtime) + Send + Sync>;

/// Script system plugin.
///
/// Loads an entry script (default `scripts/record_frame.ts`, falling back to
/// `.js`) as an ESModule graph on startup, watches the entry's directory for
/// changes and hot-reloads affected modules, and drives optional script-side
/// lifecycle hooks:
///
/// - `main()` — called once after the module graph is first evaluated.
/// - `on_input(event)` — called once per input event at the start of each
///   frame, Godot `_unhandled_input` style. Runs before `on_fixed_update`.
/// - `on_window_event(event)` — called once per window lifecycle event
///   (`close_requested`, `resized`, `focus_gained`, `focus_lost`), after
///   `on_input`. See [`window`] for the exit policy.
/// - `on_fixed_update(dt)` — called zero or more times per frame with a
///   fixed delta (default 1/60 s), Godot `_physics_process` style, for
///   framerate-independent gameplay logic. Runs before `on_update`.
/// - `on_update(dt)` — called every frame with the frame delta in seconds,
///   Unreal `Tick` / Godot `_process` style.
/// - `on_shutdown()` — called once when the app shuts down.
///
/// Missing hooks are skipped silently. Script errors are logged and do not
/// take down the app; a failed hot reload keeps the previously evaluated
/// code running and the failed batch stays pending — later updates retry it
/// (throttled) so a mid-write partial save recovers without another edit,
/// and removing a module file surfaces as a reload error rather than going
/// silent. A failed initial load is retried on the next file change (the
/// runtime and file watcher stay installed). Repeated hook
/// failures are throttled — the first few are logged in full, then a
/// periodic summary reports the count — so one buggy per-frame hook cannot
/// flood the log; the throttle resets when the hook succeeds or after a
/// successful hot reload.
///
/// The runtime is kept alive across frames in an `Rc<RefCell<_>>` (the
/// runtimes are `!Send`, so they cannot be World resources) so that
/// incremental GC and hot reload can run each frame.
pub struct ScriptPlugin {
    /// Host functions exposed to scripts, built by the embedding application.
    api: ScriptApi,
    /// Entry script path; defaults to `scripts/record_frame.ts` / `.js`.
    entry: Option<PathBuf>,
    /// Extra runtime configuration hook (e.g. registering V8 direct
    /// functions). Runs right after the runtime is created, before the entry
    /// module is loaded.
    configure: Option<ConfigureFn>,
    /// Max JS heap size in bytes; `None` uses the backend default (128 MB).
    memory_limit: Option<usize>,
    /// Fixed timestep for the `on_fixed_update` hook.
    fixed_timestep: Duration,
    /// Input state shared with the `input_*` host functions. When unset,
    /// the plugin creates its own handle (input polling then reflects only
    /// what this plugin mirrors).
    input: Option<Arc<Mutex<ScriptInputState>>>,
    /// Time state shared with the `time_*` / `frame_count` host functions.
    /// When unset, the plugin creates its own handle.
    time: Option<SharedTimeState>,
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
            fixed_timestep: Duration::from_secs_f64(1.0 / 60.0),
            input: None,
            time: None,
        }
    }

    /// Override the entry script path (default: `scripts/record_frame.ts`
    /// falling back to `scripts/record_frame.js`).
    pub fn with_entry(mut self, entry: impl Into<PathBuf>) -> Self {
        self.entry = Some(entry.into());
        self
    }

    /// Cap the JS engine's heap at `bytes` (default: 128 MB).
    pub fn with_memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit = Some(bytes);
        self
    }

    /// Set the fixed timestep for the `on_fixed_update` hook
    /// (default: 1/60 s, Godot's default physics rate).
    ///
    /// # Panics
    ///
    /// Panics if `timestep` is zero.
    pub fn with_fixed_timestep(mut self, timestep: Duration) -> Self {
        assert!(timestep > Duration::ZERO, "fixed timestep must be > 0");
        self.fixed_timestep = timestep;
        self
    }

    /// Share the [`ScriptInputState`] handle that the `input_*` host
    /// functions read. The composition root creates one handle, registers
    /// the host functions with it, and passes the same handle here so the
    /// plugin can mirror the world's `InputState` resource into it each
    /// frame.
    pub fn with_input_state(mut self, input: Arc<Mutex<ScriptInputState>>) -> Self {
        self.input = Some(input);
        self
    }

    /// Share the [`ScriptTimeState`] handle that the `time_*` / `frame_count`
    /// host functions read. The composition root creates one handle,
    /// registers the host functions with it, and passes the same handle here
    /// so the plugin can advance the frame counter each frame.
    pub fn with_time_state(mut self, time: SharedTimeState) -> Self {
        self.time = Some(time);
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
    /// Entry script path, kept so a failed initial load can be retried
    /// when a watched file changes.
    entry_path: PathBuf,
    /// Whether the entry module graph is currently loaded. False after a
    /// failed startup load until a hot-reload retry succeeds; while false,
    /// file changes retry the full entry load (the runtime has no cached
    /// registry to reload incrementally from).
    entry_loaded: bool,
    /// Wall-clock of the previous update, used to compute frame deltas.
    last_frame: Instant,
    /// Fixed-timestep accumulator feeding `on_fixed_update`.
    fixed: FixedStepAccumulator,
    /// Input state shared with the `input_*` host functions; mirrored from
    /// the world's `InputState` resource each frame.
    input: Arc<Mutex<ScriptInputState>>,
    /// Time state shared with the `time_*` / `frame_count` host functions.
    time: SharedTimeState,
    /// Throttle for repeated hook errors, so a per-frame hook that keeps
    /// failing cannot flood the log.
    error_log: HookErrorLog,
}

/// Lock the shared input state, tolerating a poisoned mutex — a panicking
/// host function must not permanently break input polling.
fn lock_input(input: &Arc<Mutex<ScriptInputState>>) -> std::sync::MutexGuard<'_, ScriptInputState> {
    input.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lock the shared time state, tolerating a poisoned mutex.
fn lock_time(time: &SharedTimeState) -> std::sync::MutexGuard<'_, ScriptTimeState> {
    time.lock().unwrap_or_else(|e| e.into_inner())
}

/// Hot-reload handler used while the entry module graph is not loaded
/// (i.e. the initial startup load failed): any script file change retries
/// the full entry load. After the first successful load the plugin
/// delegates to the runtime's own incremental [`HotReloadHandler`].
struct EntryLoadRetry<'a> {
    runtime: &'a mut Runtime,
    entry_path: &'a Path,
    /// Set once the retried load succeeds, so the plugin can switch to
    /// incremental reloads.
    succeeded: bool,
}

impl HotReloadHandler for EntryLoadRetry<'_> {
    fn on_file_changed(&mut self, changed_path: &Path) -> script::Result<()> {
        self.on_files_changed(std::slice::from_ref(&changed_path.to_path_buf()))
    }

    fn on_files_changed(&mut self, paths: &[PathBuf]) -> script::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        info!(
            "Retrying script entry '{}' after file change",
            self.entry_path.display()
        );
        load_module_entry(self.runtime, self.entry_path)?;
        self.succeeded = true;
        Ok(())
    }
}

/// Hot-reload handler delegating to the runtime's own incremental handler
/// while recording whether a reload actually ran, so the plugin can reset
/// the hook error throttles (freshly loaded code gets a clean slate).
struct ReloadReset<'a> {
    runtime: &'a mut Runtime,
    /// Set once a reload batch completed successfully.
    reloaded: bool,
}

impl HotReloadHandler for ReloadReset<'_> {
    fn on_file_changed(&mut self, changed_path: &Path) -> script::Result<()> {
        self.runtime.on_file_changed(changed_path)?;
        self.reloaded = true;
        Ok(())
    }

    fn on_files_changed(&mut self, paths: &[PathBuf]) -> script::Result<()> {
        self.runtime.on_files_changed(paths)?;
        self.reloaded = true;
        Ok(())
    }
}

/// Maximum fixed steps per frame; excess accumulated time is dropped so a
/// long stall cannot cause a catch-up ("death") spiral.
const MAX_FIXED_STEPS_PER_FRAME: u32 = 5;

/// Fixed-timestep accumulator driving the `on_fixed_update` hook
/// (Godot `_physics_process` style): real frame time is collected and
/// consumed in fixed slices, so gameplay logic runs at a stable rate
/// regardless of frame rate.
#[derive(Debug)]
struct FixedStepAccumulator {
    timestep: Duration,
    accumulated: Duration,
}

impl FixedStepAccumulator {
    fn new(timestep: Duration) -> Self {
        Self {
            timestep,
            accumulated: Duration::ZERO,
        }
    }

    /// Add one frame's elapsed time and return how many fixed steps should
    /// run this frame (0..=[`MAX_FIXED_STEPS_PER_FRAME`]). When the cap is
    /// hit the backlog is dropped — running slow beats a catch-up spiral.
    fn advance(&mut self, frame_time: Duration) -> u32 {
        self.accumulated += frame_time;
        let mut steps = (self.accumulated.as_nanos() / self.timestep.as_nanos()) as u32;
        if steps > MAX_FIXED_STEPS_PER_FRAME {
            steps = MAX_FIXED_STEPS_PER_FRAME;
            self.accumulated = Duration::ZERO;
        } else {
            self.accumulated -= self.timestep * steps;
        }
        steps
    }
}

/// Consecutive hook failures logged in full before throttling kicks in.
const HOOK_ERROR_FULL_LOGS: u64 = 3;
/// Once throttled, a summary line is logged every this many failures.
const HOOK_ERROR_SUMMARY_EVERY: u64 = 300;

/// How a hook failure should be logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookErrorVerdict {
    /// Log the full error.
    Full,
    /// Log a one-line summary carrying the total consecutive failure count.
    Summary(u64),
    /// Suppress this occurrence.
    Suppress,
}

/// Throttles repeated hook errors so a per-frame hook that keeps failing
/// cannot flood the log: the first few consecutive failures are logged in
/// full, afterwards a periodic summary reports the count. A hook success
/// resets its counter, and a successful hot reload resets all of them —
/// freshly loaded code gets a clean slate.
#[derive(Debug, Default)]
struct HookErrorLog {
    /// Consecutive failure count per hook.
    failures: HashMap<&'static str, u64>,
}

impl HookErrorLog {
    /// Record a hook failure and decide how it should be logged.
    fn fail(&mut self, hook: &'static str) -> HookErrorVerdict {
        let count = self.failures.entry(hook).or_insert(0);
        *count += 1;
        let count = *count;
        if count <= HOOK_ERROR_FULL_LOGS {
            HookErrorVerdict::Full
        } else if count.is_multiple_of(HOOK_ERROR_SUMMARY_EVERY) {
            HookErrorVerdict::Summary(count)
        } else {
            HookErrorVerdict::Suppress
        }
    }

    /// Record and log a hook failure according to the throttle.
    fn log_failure(&mut self, hook: &'static str, error: &str) {
        match self.fail(hook) {
            HookErrorVerdict::Full => error!("script {} failed: {}", hook, error),
            HookErrorVerdict::Summary(count) => error!(
                "script {} still failing ({} consecutive failures); latest: {}",
                hook, count, error
            ),
            HookErrorVerdict::Suppress => {}
        }
    }

    /// Reset one hook's throttle (the hook succeeded).
    fn reset(&mut self, hook: &'static str) {
        self.failures.remove(&hook);
    }

    /// Reset all throttles (a successful hot reload brought in fresh code).
    fn reset_all(&mut self) {
        self.failures.clear();
    }
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

        let mut api = self.api.clone();
        let time = self.time.clone().unwrap_or_else(new_shared_time);
        register_time_api(&mut api, &time);
        let entry = self.entry.clone();
        let configure = self.configure.clone();
        let memory_limit = self.memory_limit;
        let fixed_timestep = self.fixed_timestep;
        let input = self
            .input
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(ScriptInputState::default())));
        let slot = state_slot.clone();
        let startup_time = Arc::clone(&time);
        app.add_startup_system(move |_world: &mut World| {
            info!("Script plugin startup");
            let startup = Instant::now();
            lock_time(&startup_time).set_startup(startup);
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
            // A failed initial load (e.g. one syntax error) must not kill
            // scripting: the state is installed below regardless, and the
            // hot-reload path retries the entry load on the next file
            // change (see `EntryLoadRetry`).
            let entry_loaded = match load_module_entry(&mut runtime, &entry_path) {
                Ok(_) => true,
                Err(e) => {
                    error!("Failed to load script '{}': {}", entry_path.display(), e);
                    false
                }
            };
            // Watch the entry's directory for hot reload. `.ts` sources are
            // transpiled in-process on (re)load, so edits take effect
            // directly.
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
                entry_path,
                entry_loaded,
                last_frame: startup,
                fixed: FixedStepAccumulator::new(fixed_timestep),
                input,
                time,
                error_log: HookErrorLog::default(),
            });
        });

        let slot = state_slot.clone();
        app.add_update_system(move |world: &mut World| {
            let mut slot = slot.borrow_mut();
            let Some(state) = slot.as_mut() else {
                return true;
            };
            state.runtime.gc_step();
            if let Some(reloader) = state.hot_reloader.as_mut() {
                // While the entry graph is missing (failed startup load),
                // any file change retries the full entry load; otherwise
                // the runtime reloads the changed modules incrementally.
                // A successful reload resets the hook error throttles —
                // freshly loaded code gets a clean slate.
                let result = if state.entry_loaded {
                    let mut reload = ReloadReset {
                        runtime: &mut state.runtime,
                        reloaded: false,
                    };
                    let result = reloader.poll(&mut reload);
                    if reload.reloaded {
                        state.error_log.reset_all();
                    }
                    result
                } else {
                    let mut retry = EntryLoadRetry {
                        runtime: &mut state.runtime,
                        entry_path: &state.entry_path,
                        succeeded: false,
                    };
                    let result = reloader.poll(&mut retry);
                    state.entry_loaded = retry.succeeded;
                    if retry.succeeded {
                        state.error_log.reset_all();
                    }
                    result
                };
                if let Err(e) = result {
                    error!("Hot reload failed: {}", e);
                }
            }
            let now = Instant::now();
            let frame_time = now.duration_since(state.last_frame);
            state.last_frame = now;
            lock_time(&state.time).increment_frame();

            // Input phase: mirror the world's InputState into the shared
            // script-facing state, then replay this frame's events to the
            // `on_input` hook (before any fixed steps — "input first, then
            // simulation", as in Unreal/Godot/Bevy). Events are iterated by
            // reference (no per-frame clone) and only when the hook exists.
            if let Some(input) = world.get_resource::<moonfield_window::InputState>() {
                lock_input(&state.input).sync_frame(&input);
                if state.runtime.has_function("on_input") {
                    for event in input.events() {
                        let arg = input::input_event_to_host(event);
                        match state.runtime.call_module_export_unit("on_input", &[arg]) {
                            Ok(()) => state.error_log.reset("on_input"),
                            Err(e) => {
                                state.error_log.log_failure("on_input", &e.to_string());
                                break;
                            }
                        }
                    }
                }
            }

            // Window lifecycle events (close/resize/focus) — a separate
            // channel from gameplay input, also replayed before any fixed
            // steps.
            if let Some(events) = world.get_resource::<moonfield_window::WindowEvents>() {
                if state.runtime.has_function("on_window_event") {
                    for event in events.events() {
                        let arg = window::window_event_to_host(event);
                        match state
                            .runtime
                            .call_module_export_unit("on_window_event", &[arg])
                        {
                            Ok(()) => state.error_log.reset("on_window_event"),
                            Err(e) => {
                                state
                                    .error_log
                                    .log_failure("on_window_event", &e.to_string());
                                break;
                            }
                        }
                    }
                }
            }

            // Fixed-timestep gameplay hook (Godot `_physics_process` /
            // Unity `FixedUpdate` style), runs before `on_update`.
            if state.runtime.has_function("on_fixed_update") {
                let dt = fixed_timestep.as_secs_f64();
                for _ in 0..state.fixed.advance(frame_time) {
                    lock_input(&state.input).begin_fixed_step();
                    match state
                        .runtime
                        .call_module_export_unit("on_fixed_update", &[HostValue::Number(dt)])
                    {
                        Ok(()) => {
                            state.error_log.reset("on_fixed_update");
                            lock_input(&state.input).end_fixed_step();
                        }
                        Err(e) => {
                            state
                                .error_log
                                .log_failure("on_fixed_update", &e.to_string());
                            lock_input(&state.input).cancel_fixed_step();
                            break;
                        }
                    }
                }
            }

            let dt = frame_time.as_secs_f64();
            if state.runtime.has_function("on_update") {
                match state
                    .runtime
                    .call_module_export_unit("on_update", &[HostValue::Number(dt)])
                {
                    Ok(()) => state.error_log.reset("on_update"),
                    Err(e) => state.error_log.log_failure("on_update", &e.to_string()),
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

/// Resolve the default entry script: `scripts/record_frame.ts` if present,
/// else `scripts/record_frame.js`. The `.ts` source wins when both exist —
/// a checked-in or stale compiled `.js` must never shadow it.
fn default_script_path() -> PathBuf {
    let script_dir = Path::new("scripts");
    let ts_path = script_dir.join("record_frame.ts");
    if ts_path.exists() {
        ts_path
    } else {
        script_dir.join("record_frame.js")
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed initial entry load (e.g. a syntax error) must not be
    /// permanent: the retry handler the plugin wires into hot reload picks
    /// up the fixed file and its exports become callable.
    #[test]
    fn failed_initial_load_retries_on_file_change() {
        let dir = std::env::temp_dir().join(format!(
            "moonfield_script_test_entry_retry_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let entry = dir.join("main.ts");
        std::fs::write(&entry, "export function value( { not valid").unwrap();

        let mut runtime = Runtime::new(ScriptApi::default()).expect("runtime");
        // The startup load fails — previously the plugin gave up here and
        // never installed the runtime or the file watcher.
        assert!(load_module_entry(&mut runtime, &entry).is_err());
        assert!(!runtime.has_function("value"));

        // Fix the file, then drive the same retry path the plugin's
        // hot-reload polling uses while the entry is not loaded.
        std::fs::write(&entry, "export function value(): number { return 42; }").unwrap();
        let mut retry = EntryLoadRetry {
            runtime: &mut runtime,
            entry_path: &entry,
            succeeded: false,
        };
        retry
            .on_files_changed(std::slice::from_ref(&entry))
            .expect("retry after fix");
        assert!(retry.succeeded);
        assert!(runtime.has_function("value"));
        let v = runtime
            .call_module_export("value", &[])
            .expect("call value");
        assert_eq!(v.as_f64(), Some(42.0));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fixed_step_accumulator_slices_frame_time() {
        let mut acc = FixedStepAccumulator::new(Duration::from_millis(10));
        assert_eq!(acc.advance(Duration::from_millis(25)), 2); // 5ms carried over
        assert_eq!(acc.advance(Duration::from_millis(4)), 0); // 9ms total
        assert_eq!(acc.advance(Duration::from_millis(1)), 1); // 10ms — one step
        assert_eq!(acc.advance(Duration::ZERO), 0);
    }

    #[test]
    fn fixed_step_accumulator_caps_and_drops_backlog() {
        let mut acc = FixedStepAccumulator::new(Duration::from_millis(10));
        // A 500ms stall yields far more than the cap: clamp, then drop the
        // backlog so the next frame starts clean.
        assert_eq!(
            acc.advance(Duration::from_millis(500)),
            MAX_FIXED_STEPS_PER_FRAME
        );
        assert_eq!(acc.advance(Duration::ZERO), 0);
        assert_eq!(acc.advance(Duration::from_millis(10)), 1);
    }

    #[test]
    fn hook_error_log_throttles_repeated_failures() {
        let mut log = HookErrorLog::default();
        // The first failures are logged in full.
        for _ in 0..HOOK_ERROR_FULL_LOGS {
            assert_eq!(log.fail("on_update"), HookErrorVerdict::Full);
        }
        // Then suppressed until the periodic summary kicks in.
        for _ in HOOK_ERROR_FULL_LOGS..(HOOK_ERROR_SUMMARY_EVERY - 1) {
            assert_eq!(log.fail("on_update"), HookErrorVerdict::Suppress);
        }
        assert_eq!(
            log.fail("on_update"),
            HookErrorVerdict::Summary(HOOK_ERROR_SUMMARY_EVERY)
        );
        // Hooks are throttled independently.
        assert_eq!(log.fail("on_fixed_update"), HookErrorVerdict::Full);
    }

    #[test]
    fn hook_error_log_resets_on_success_and_hot_reload() {
        let mut log = HookErrorLog::default();
        for _ in 0..=HOOK_ERROR_FULL_LOGS {
            log.fail("on_update");
        }
        assert_eq!(log.fail("on_update"), HookErrorVerdict::Suppress);
        // A success resets that hook: the next failure is full again.
        log.reset("on_update");
        assert_eq!(log.fail("on_update"), HookErrorVerdict::Full);
        // A hot reload resets every hook.
        for _ in 0..=HOOK_ERROR_FULL_LOGS {
            log.fail("on_fixed_update");
        }
        log.reset_all();
        assert_eq!(log.fail("on_fixed_update"), HookErrorVerdict::Full);
        // Resetting a hook that never failed is a no-op.
        log.reset("on_input");
    }
}

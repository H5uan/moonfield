//! V8 backend for the scripting runtime.

use super::source_map::SourceMapCache;
use super::{
    HostFn, HostValue, HotReloadHandler, Result, ScriptApi, ScriptError, ScriptRuntime,
    TypedArrayElement, TypedArrayValue, DEFAULT_EXECUTION_TIMEOUT,
};
use moonfield_log::{error, info, warn};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

static V8_INIT: Once = Once::new();

/// Cached V8 startup snapshot containing the console JS shim.
/// Created once on the first `V8Runtime::new()` call, reused by all
/// subsequent isolates — avoids re-parsing/re-compiling the console
/// shim on every isolate creation.
static SNAPSHOT_BLOB: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// Create a V8 startup snapshot containing the console JS shim.
///
/// The snapshot captures the heap state (including the `console` object
/// with its JS methods). Native functions (`__mf_log`, host APIs) are
/// NOT in the snapshot — they must be re-registered after loading.
fn create_console_snapshot() -> Vec<u8> {
    let mut isolate = v8::Isolate::snapshot_creator(None, None);
    {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        // Eval console shim so the snapshot includes the console object.
        let code = v8::String::new(scope, V8Runtime::CONSOLE_SHIM).unwrap();
        if let Some(script) = v8::Script::compile(scope, code, None) {
            let _ = script.run(scope);
        }
        scope.set_default_context(context);
    }
    // Keep the shim's compiled code in the snapshot so runtime isolates
    // don't recompile it on startup.
    match isolate.create_blob(v8::FunctionCodeHandling::Keep) {
        Some(data) => data.to_vec(),
        None => Vec::new(),
    }
}

/// A V8-native host function that operates on raw V8 types.
///
/// Bypasses the `HostValue` marshaling layer for direct Rust↔V8 communication.
/// Use for high-frequency functions where the overhead of
/// `v8_value_to_host` → `HostValue` → typed extraction is measurable.
pub type DirectFn = fn(&mut v8::PinScope, v8::FunctionCallbackArguments, v8::ReturnValue);

/// A direct (fast-path) host function plus its registration name.
///
/// Boxed and referenced through `v8::External` so the (zero-sized,
/// non-capturing) V8 callback can recover both the function pointer and the
/// name for error reporting.
struct DirectFnEntry {
    name: &'static str,
    func: DirectFn,
}

/// A `HostValue`-marshaling host function plus its registration name.
///
/// Boxed for a stable address so the `v8::External` data pointer handed to
/// V8 can never dangle (same pattern as [`DirectFnEntry`]).
struct ApiFnEntry {
    name: &'static str,
    func: HostFn,
}

/// A script runtime backed by V8 (rusty_v8).
///
/// Module system: uses V8's native ESModule API via `ScriptCompiler::compile_module`
/// and `Module::instantiate_module2`. The resolve callback is a static method that
/// reads a `ModuleMap` pointer from the isolate's data slot (set before
/// `instantiate_module2` and removed after). This pattern follows Deno's approach.
///
/// Hot reload: stores the last [`ModuleRegistry`] and entry point so that
/// [`HotReloadHandler::on_file_changed`] can recompile only the changed module
/// and its transitive dependents, reusing cached compiled modules for the rest.
///
/// GC control: heap limits are set at creation time to prevent unbounded growth.
/// [`gc_step`] signals idle to V8, allowing incremental GC in background threads.
/// Script entry points (`call`, `load`, `load_module_graph`) call `set_idle(false)`
/// to resume active mode before executing JS.
///
/// Fast path: high-frequency host functions can be registered via [`register_direct`]
/// to bypass the `HostValue` marshaling layer and operate on V8 types directly.
pub struct V8Runtime {
    /// Terminates runaway script executions after `execution_timeout`.
    ///
    /// Declared first so it is dropped first: its thread is stopped and
    /// joined before the isolate it references is destroyed.
    watchdog: ExecutionWatchdog,
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    /// Host functions exposed to scripts, one box per entry.
    ///
    /// V8 callbacks receive a raw `External` pointer to their entry, so each
    /// entry is boxed for a stable address — storing plain tuples in the
    /// `Vec` would let a reallocation dangle every installed callback.
    #[allow(clippy::vec_box)] // boxes required for pointer stability, see above
    api_entries: Vec<Box<ApiFnEntry>>,
    /// Direct (fast-path) host functions that bypass `HostValue` marshaling.
    ///
    /// Each entry is boxed so that `v8::External` pointers handed to V8 stay
    /// valid even when the vector grows (the heap allocation does not move).
    #[allow(clippy::vec_box)] // box required for pointer stability, see above
    direct_fns: Vec<Box<DirectFnEntry>>,
    /// Cached registry for hot reload (populated by `load_module_graph`).
    /// Stored as `Rc` to avoid cloning all module source strings on every
    /// hot reload cycle. `on_file_changed` uses `Rc::try_unwrap` to regain
    /// ownership (cheap, since only one ref exists after `take()`).
    registry: Option<Rc<super::ModuleRegistry>>,
    /// Cached entry point for hot reload.
    entry: Option<String>,
    /// Cached compiled modules for incremental hot reload.
    /// Keyed by canonical module name. Source is stored for change detection.
    compiled_modules: HashMap<String, CachedModule>,
    /// Cached module namespace for `call_module_export`.
    module_namespace: Option<v8::Global<v8::Object>>,
    /// Resolved module-export functions, keyed by name. Avoids re-allocating
    /// the name string and re-walking the namespace object on every call
    /// (the per-frame `on_update` hot path). Cleared whenever the module
    /// graph is (re)evaluated or the context is recreated.
    function_cache: HashMap<String, v8::Global<v8::Function>>,
    /// Per-execution time limit armed at every top-level entry point.
    execution_timeout: Duration,
    /// Source maps for remapping error locations back to TypeScript.
    source_maps: SourceMapCache,
}

impl ScriptRuntime for V8Runtime {
    fn new(api: ScriptApi) -> Result<Self> {
        Self::new_with_memory_limit(api, super::DEFAULT_MAX_HEAP_BYTES)
    }

    fn load(&mut self, name: &str, source: &str) -> Result<()> {
        self.isolate.set_idle(false);
        // Clear any stale termination from a previously terminated call, then
        // arm the watchdog so a runaway execution gets interrupted.
        self.isolate.cancel_terminate_execution();
        let _watchdog_guard = self.watchdog.arm(self.execution_timeout);
        // Best-effort: pick up a source map so error locations point back to
        // the original TypeScript.
        self.source_maps.load_for(name, source);
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        v8::tc_scope!(let tc, scope);

        let name_str = v8::String::new(tc, name).unwrap();
        let origin = v8::ScriptOrigin::new(
            tc,
            name_str.into(),
            0,
            0,
            false,
            0,
            None,
            false,
            false,
            false, // is_module: false — plain script
            None,
        );
        let code = v8::String::new(tc, source)
            .ok_or_else(|| ScriptError::Execution("failed to create source string".into()))?;
        let script = match v8::Script::compile(tc, code, Some(&origin)) {
            Some(s) => s,
            None => return Err(ScriptError::Execution(v8_exception!(tc, &self.source_maps))),
        };
        match script.run(tc) {
            Some(_) => Ok(()),
            None => Err(ScriptError::Execution(v8_exception!(tc, &self.source_maps))),
        }
    }

    fn reload(&mut self) -> Result<()> {
        let context = {
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Context::new(handle_scope, Default::default());
            v8::Global::new(handle_scope, context)
        };
        self.context = context;
        // Handles into the old context are invalid now — drop module state so
        // `has_function`/`call_module_export` can't touch stale globals.
        self.module_namespace = None;
        self.function_cache.clear();
        self.compiled_modules.clear();
        self.source_maps.clear();
        self.register_api()
    }

    fn call(&mut self, function: &str) -> Result<()> {
        self.isolate.set_idle(false);
        // Clear any stale termination from a previously terminated call, then
        // arm the watchdog so a runaway execution gets interrupted.
        self.isolate.cancel_terminate_execution();
        let _watchdog_guard = self.watchdog.arm(self.execution_timeout);
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        v8::tc_scope!(let tc, scope);

        let global = local_context.global(tc);
        let name = v8::String::new(tc, function).unwrap();
        let value = global
            .get(tc, name.into())
            .ok_or_else(|| ScriptError::Runtime(format!("function '{}' not found", function)))?;
        let func = v8::Local::<v8::Function>::try_from(value)
            .map_err(|_| ScriptError::Runtime(format!("'{}' is not a function", function)))?;
        let recv = v8::undefined(tc);
        if func.call(tc, recv.into(), &[]).is_none() {
            return Err(ScriptError::Runtime(format!(
                "{}: {}",
                function,
                v8_exception!(tc, &self.source_maps)
            )));
        }
        // Drain the microtask queue so promises settled by this call
        // (`.then` callbacks, async continuations) run before returning.
        tc.perform_microtask_checkpoint();
        Ok(())
    }

    fn call_with_args(&mut self, function: &str, args: &[HostValue]) -> Result<HostValue> {
        self.isolate.set_idle(false);
        // Clear any stale termination from a previously terminated call, then
        // arm the watchdog so a runaway execution gets interrupted.
        self.isolate.cancel_terminate_execution();
        let _watchdog_guard = self.watchdog.arm(self.execution_timeout);
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        v8::tc_scope!(let tc, scope);

        let global = local_context.global(tc);
        let name = v8::String::new(tc, function).unwrap();
        let value = global
            .get(tc, name.into())
            .ok_or_else(|| ScriptError::Runtime(format!("function '{}' not found", function)))?;
        let func = v8::Local::<v8::Function>::try_from(value)
            .map_err(|_| ScriptError::Runtime(format!("'{}' is not a function", function)))?;

        let v8_args: Vec<v8::Local<v8::Value>> =
            args.iter().map(|a| host_to_v8_value(a, tc)).collect();

        let recv = v8::undefined(tc);
        match func.call(tc, recv.into(), &v8_args) {
            Some(result) => {
                let host = v8_value_to_host(result, tc);
                // Drain the microtask queue so settled promises run.
                tc.perform_microtask_checkpoint();
                Ok(host)
            }
            None => Err(ScriptError::Runtime(format!(
                "{}: {}",
                function,
                v8_exception!(tc, &self.source_maps)
            ))),
        }
    }

    fn call_module_export(&mut self, function: &str, args: &[HostValue]) -> Result<HostValue> {
        self.call_module_export_impl(function, args, true)
            .map(|v| v.unwrap_or(HostValue::Null))
    }

    fn call_module_export_unit(&mut self, function: &str, args: &[HostValue]) -> Result<()> {
        self.call_module_export_impl(function, args, false)
            .map(|_| ())
    }

    fn gc_step(&mut self) {
        // Signal V8 that the isolate is idle. V8's background GC thread can
        // do incremental sweeping during the gap between frames.
        // `set_idle(false)` is called by `call`/`load`/`load_module_graph`
        // before any JS execution resumes active mode.
        self.isolate.set_idle(true);
    }

    fn has_function(&mut self, name: &str) -> bool {
        // Fast path: previously resolved module export (the per-frame
        // lifecycle hooks hit this after their first call).
        if self.function_cache.contains_key(name) {
            return true;
        }
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        let Some(key) = v8::String::new(scope, name) else {
            return false;
        };
        // Prefer the module namespace (entry exports) when a module graph is
        // loaded; otherwise fall back to globals.
        let target = match &self.module_namespace {
            Some(ns) => v8::Local::new(scope, ns),
            None => local_context.global(scope),
        };
        let Some(value) = target.get(scope, key.into()) else {
            return false;
        };
        if !value.is_function() {
            return false;
        }
        // Cache resolved module exports so subsequent lookups skip the string
        // allocation and property walk. Globals are not cached — they can be
        // redefined by later `load()` calls.
        if self.module_namespace.is_some() {
            if let Ok(func) = v8::Local::<v8::Function>::try_from(value) {
                self.function_cache
                    .insert(name.to_string(), v8::Global::new(scope, func));
            }
        }
        true
    }

    fn load_module_graph(
        &mut self,
        registry: Rc<super::ModuleRegistry>,
        entry: &str,
    ) -> Result<()> {
        V8Runtime::load_module_graph(self, registry, entry)
    }
}

impl V8Runtime {
    /// Create a runtime with an explicit maximum heap size (in bytes).
    ///
    /// [`ScriptRuntime::new`] uses [`DEFAULT_MAX_HEAP_BYTES`]; the initial
    /// heap is fixed at 4 MB and grows up to `max_bytes`.
    pub fn new_with_memory_limit(api: ScriptApi, max_bytes: usize) -> Result<Self> {
        V8_INIT.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });

        // Ensure the console snapshot exists (first call creates it).
        // The OnceLock lives forever, so the slice can be borrowed as
        // 'static — no per-isolate copy of the blob.
        let snapshot_bytes: &'static [u8] = SNAPSHOT_BLOB.get_or_init(create_console_snapshot);

        // Set heap limits and use the cached snapshot if available.
        let create_params = v8::CreateParams::default().heap_limits(4 * 1024 * 1024, max_bytes);
        let create_params = if !snapshot_bytes.is_empty() {
            create_params.snapshot_blob(v8::StartupData::from(snapshot_bytes))
        } else {
            create_params
        };

        let mut isolate = v8::Isolate::new(create_params);
        let watchdog = ExecutionWatchdog::new(&isolate);

        let context = {
            v8::scope!(let handle_scope, &mut isolate);
            let context = v8::Context::new(handle_scope, Default::default());
            v8::Global::new(handle_scope, context)
        };

        let api_entries = api
            .iter()
            .map(|entry| {
                Box::new(ApiFnEntry {
                    name: entry.0,
                    func: entry.1.clone(),
                })
            })
            .collect();

        let mut rt = Self {
            isolate,
            context,
            api_entries,
            direct_fns: Vec::new(),
            registry: None,
            entry: None,
            compiled_modules: HashMap::new(),
            module_namespace: None,
            function_cache: HashMap::new(),
            watchdog,
            execution_timeout: DEFAULT_EXECUTION_TIMEOUT,
            source_maps: SourceMapCache::new(),
        };
        rt.register_api()?;
        Ok(rt)
    }

    /// Set the per-execution time limit for top-level script calls.
    ///
    /// A call that runs longer than this (e.g. an infinite loop) is
    /// terminated by the watchdog thread and returns an error; the isolate
    /// stays usable for subsequent calls.
    pub fn set_execution_timeout(&mut self, timeout: Duration) {
        self.execution_timeout = timeout;
    }

    /// Register a fast-path host function that operates on V8 types directly.
    ///
    /// Bypasses `HostValue` marshaling. The binding is installed into the
    /// current context immediately (overwriting any `HostFn` binding with the
    /// same name) and re-installed on every `reload()`. Registering the same
    /// name twice is a no-op.
    pub fn register_direct(&mut self, name: &'static str, func: DirectFn) {
        if self.direct_fns.iter().any(|e| e.name == name) {
            return;
        }
        self.direct_fns.push(Box::new(DirectFnEntry { name, func }));
        let entry = self.direct_fns.last().unwrap();
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        // The context always exists here (created in `new`), so immediate
        // installation cannot fail in a way worth surfacing — log instead.
        if let Err(e) = Self::install_direct(scope, local_context, entry) {
            error!("register_direct('{}'): {}", name, e);
        }
    }

    /// Install one direct-function binding into the context's global object.
    fn install_direct(
        scope: &mut v8::PinScope,
        context: v8::Local<v8::Context>,
        entry: &DirectFnEntry,
    ) -> Result<()> {
        let global = context.global(scope);
        let js_name = v8::String::new(scope, entry.name).unwrap();
        // The pointer targets the boxed entry owned by `direct_fns`; the
        // box's heap allocation never moves while V8 callbacks are live,
        // even if the vector grows.
        let ptr = entry as *const DirectFnEntry as *mut std::ffi::c_void;
        let external = v8::External::new(scope, ptr);
        // NOTE: the callback must be a non-capturing closure — V8 requires
        // function callbacks to be zero-sized. All state flows through the
        // `External` data pointer.
        let js_func = v8::Function::builder(
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, r: v8::ReturnValue| {
                let Ok(external) = v8::Local::<v8::External>::try_from(a.data()) else {
                    return;
                };
                // Safety: the pointer references a `Box<DirectFnEntry>` owned
                // by this runtime's `direct_fns`, which outlives the context.
                let entry = unsafe { &*(external.value() as *const DirectFnEntry) };
                // A panic in the host function must not unwind through V8's
                // C++ frames (that is undefined behavior) — catch it and
                // surface it as a JS exception instead.
                if let Err(payload) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (entry.func)(s, a, r)))
                {
                    throw_host_error(
                        s,
                        &format!(
                            "host function '{}' panicked: {}",
                            entry.name,
                            super::panic_payload_message(&payload)
                        ),
                    );
                }
            },
        )
        .data(external.into())
        .build(scope)
        .ok_or_else(|| ScriptError::Runtime(format!("failed to build direct {}", entry.name)))?;
        global.set(scope, js_name.into(), js_func.into()).unwrap();
        Ok(())
    }

    fn register_api(&mut self) -> Result<()> {
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        let global = local_context.global(scope);

        // Register direct (fast-path) functions first — no HostValue marshaling.
        for entry in &self.direct_fns {
            Self::install_direct(scope, local_context, entry)?;
        }

        // Register regular HostFn-based functions.
        for entry in &self.api_entries {
            // Skip if already registered as a direct function.
            if self.direct_fns.iter().any(|d| d.name == entry.name) {
                continue;
            }

            // Stable pointer to the boxed entry (the box never moves).
            let ptr = &**entry as *const ApiFnEntry as *mut std::ffi::c_void;
            let data = v8::External::new(scope, ptr);
            let js_name = v8::String::new(scope, entry.name).unwrap();
            let func = v8::Function::builder(
                |scope: &mut v8::PinScope,
                 args: v8::FunctionCallbackArguments,
                 mut retval: v8::ReturnValue| {
                    let Ok(external) = v8::Local::<v8::External>::try_from(args.data()) else {
                        return;
                    };
                    // Safety: the pointer references a `Box<ApiFnEntry>`
                    // owned by this runtime's `api_entries`, which outlives
                    // the context.
                    let entry = unsafe { &*(external.value() as *const ApiFnEntry) };
                    let name = entry.name;

                    // A panic anywhere in arg marshaling or the host function
                    // must not unwind through V8's C++ frames (that is
                    // undefined behavior) — catch it and surface it as a JS
                    // exception instead.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // Marshal JS arguments → HostValue slice.
                        let n = args.length() as usize;
                        let mut host_args: Vec<HostValue> = Vec::with_capacity(n);
                        for i in 0..n as i32 {
                            host_args.push(v8_value_to_host(args.get(i), scope));
                        }
                        (entry.func)(&host_args)
                    }));

                    match result {
                        Ok(Ok(ret)) => {
                            retval.set(host_to_v8_value(&ret, scope));
                        }
                        Ok(Err(e)) => throw_host_error(scope, &e),
                        Err(payload) => throw_host_error(
                            scope,
                            &format!(
                                "host function '{}' panicked: {}",
                                name,
                                super::panic_payload_message(&payload)
                            ),
                        ),
                    }
                },
            )
            .data(data.into())
            .build(scope)
            .ok_or_else(|| ScriptError::Runtime(format!("failed to build {}", entry.name)))?;
            global.set(scope, js_name.into(), func.into()).unwrap();
        }

        Self::register_console(scope, local_context)
    }

    /// JS console shim that delegates to `__mf_log(level, msg)`.
    /// Same approach as QuickJS — enables snapshot serialization (native
    /// Function callbacks can't be snapshotted, but JS closures can).
    const CONSOLE_SHIM: &'static str = r#"
globalThis.console = {
  log:   function() { __mf_log(0, Array.prototype.map.call(arguments, String).join(" ")); },
  info:  function() { __mf_log(0, Array.prototype.map.call(arguments, String).join(" ")); },
  warn:  function() { __mf_log(1, Array.prototype.map.call(arguments, String).join(" ")); },
  error: function() { __mf_log(2, Array.prototype.map.call(arguments, String).join(" ")); }
};
"#;

    /// Register `__mf_log` native function and console JS shim.
    ///
    /// If the isolate was created from a snapshot containing the console
    /// shim, the shim eval is a no-op (overwrites with identical object).
    fn register_console(scope: &mut v8::PinScope, context: v8::Local<v8::Context>) -> Result<()> {
        let global = context.global(scope);

        // Register native __mf_log that forwards to host logger.
        let log_fn = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                let level = a.get(0).uint32_value(s).unwrap_or(0);
                let msg = a.get(1).to_rust_string_lossy(s);
                match level {
                    0 => info!("{}", msg),
                    1 => warn!("{}", msg),
                    _ => error!("{}", msg),
                }
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build __mf_log".into()))?;
        let log_name = v8::String::new(scope, "__mf_log").unwrap();
        global.set(scope, log_name.into(), log_fn.into());

        // Eval console shim (idempotent — overwrites if already from snapshot).
        let code = v8::String::new(scope, Self::CONSOLE_SHIM).unwrap();
        let script = v8::Script::compile(scope, code, None)
            .ok_or_else(|| ScriptError::Runtime("failed to compile console shim".into()))?;
        script.run(scope);
        Ok(())
    }
}

impl V8Runtime {
    /// Shared implementation of `call_module_export` / `call_module_export_unit`.
    ///
    /// Resolved export functions are cached as `Global<Function>` (keyed by
    /// name) so the per-frame hot path avoids re-allocating the name string
    /// and re-walking the namespace object. When `marshal_result` is false
    /// the return value is discarded without converting it to a `HostValue`.
    fn call_module_export_impl(
        &mut self,
        function: &str,
        args: &[HostValue],
        marshal_result: bool,
    ) -> Result<Option<HostValue>> {
        if self.module_namespace.is_none() {
            return Err(ScriptError::Runtime("no module namespace loaded".into()));
        }
        self.isolate.set_idle(false);
        // Clear any stale termination from a previously terminated call, then
        // arm the watchdog so a runaway execution gets interrupted.
        self.isolate.cancel_terminate_execution();
        let _watchdog_guard = self.watchdog.arm(self.execution_timeout);
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        v8::tc_scope!(let tc, scope);

        // Resolve the export, preferring the cached function handle.
        let func = match self.function_cache.get(function) {
            Some(cached) => v8::Local::new(tc, cached),
            None => {
                let namespace = self.module_namespace.as_ref().unwrap();
                let ns = v8::Local::new(tc, namespace);
                let name = v8::String::new(tc, function)
                    .ok_or_else(|| ScriptError::Runtime("export name too long".into()))?;
                let value = ns.get(tc, name.into()).ok_or_else(|| {
                    ScriptError::Runtime(format!("export '{}' not found", function))
                })?;
                let func = v8::Local::<v8::Function>::try_from(value).map_err(|_| {
                    ScriptError::Runtime(format!("export '{}' is not a function", function))
                })?;
                self.function_cache
                    .insert(function.to_string(), v8::Global::new(tc, func));
                func
            }
        };

        let v8_args: Vec<v8::Local<v8::Value>> =
            args.iter().map(|a| host_to_v8_value(a, tc)).collect();

        let recv = v8::undefined(tc);
        match func.call(tc, recv.into(), &v8_args) {
            Some(result) => {
                let host = marshal_result.then(|| v8_value_to_host(result, tc));
                // Drain the microtask queue so settled promises run.
                tc.perform_microtask_checkpoint();
                Ok(host)
            }
            None => Err(ScriptError::Runtime(format!(
                "{}: {}",
                function,
                v8_exception!(tc, &self.source_maps)
            ))),
        }
    }

    /// Load a module graph using V8's native ESModule API.
    ///
    /// 1. Compiles modules via `ScriptCompiler::compile_module`, caching
    ///    compiled handles in `self.compiled_modules` for incremental reload.
    /// 2. Stores compiled `Local<Module>` handles in a `ModuleMap`.
    /// 3. Stores a `*const ModuleMap` pointer in the isolate slot.
    /// 4. Calls `Module::instantiate_module2` with a static resolve callback.
    /// 5. Evaluates the entry module and calls `exports.main()`.
    ///
    /// On reload, modules whose source hasn't changed reuse cached compiled
    /// handles (already evaluated by V8). Only modules removed from the cache
    /// (by `on_file_changed`) are re-compiled.
    pub fn load_module_graph(
        &mut self,
        registry: Rc<super::ModuleRegistry>,
        entry: &str,
    ) -> Result<()> {
        // The graph is about to be re-evaluated: previously resolved export
        // functions may belong to stale module instances, so drop them and
        // let calls re-resolve lazily.
        self.function_cache.clear();

        let order = registry
            .order_dependencies(entry)
            .map_err(|e| ScriptError::Execution(format!("dependency resolution: {}", e)))?;

        self.isolate.set_idle(false);
        // Clear any stale termination from a previously terminated call, then
        // arm the watchdog so a runaway execution gets interrupted.
        self.isolate.cancel_terminate_execution();
        let _watchdog_guard = self.watchdog.arm(self.execution_timeout);
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);

        // Step 1: compile modules, reusing cached handles where source matches.
        let mut module_map = ModuleMap {
            modules: HashMap::new(),
            by_hash: HashMap::new(),
            resolutions: HashMap::new(),
            names: order.clone(),
        };

        for module_name in &order {
            let info = registry.get(module_name).ok_or_else(|| {
                ScriptError::Execution(format!("module '{}' not found", module_name))
            })?;

            // Check if a cached compiled module with matching source exists.
            let cached = self.compiled_modules.get(module_name);
            let reuse = cached.is_some_and(|c| c.source == info.source);

            if reuse {
                // Reuse cached compiled module (already evaluated by V8).
                let module_global = cached.unwrap().module.clone();
                module_map.modules.insert(
                    module_name.clone(),
                    ModuleEntry {
                        module: module_global,
                        namespace: None,
                    },
                );
            } else {
                // Compile from source and cache.
                let code = v8::String::new(scope, &info.source).ok_or_else(|| {
                    ScriptError::Execution("failed to create source string".into())
                })?;
                let origin = v8::ScriptOrigin::new(
                    scope,
                    v8::String::new(scope, module_name).unwrap().into(),
                    0,
                    0,
                    false,
                    0,
                    None,
                    false,
                    false,
                    true,
                    None,
                );
                let mut source = v8::script_compiler::Source::new(code, Some(&origin));
                let module =
                    v8::script_compiler::compile_module(scope, &mut source).ok_or_else(|| {
                        let msg = format!("failed to compile module '{}'", module_name);
                        ScriptError::Execution(msg)
                    })?;

                // Best-effort: pick up the module's source map so error
                // locations point back to the original TypeScript.
                self.source_maps.load_for(module_name, &info.source);

                let module_global = v8::Global::new(scope, module);
                self.compiled_modules.insert(
                    module_name.clone(),
                    CachedModule {
                        module: module_global.clone(),
                        source: info.source.clone(),
                    },
                );
                module_map.modules.insert(
                    module_name.clone(),
                    ModuleEntry {
                        module: module_global,
                        namespace: None,
                    },
                );
            }
        }

        // Step 2: get the entry module handle.
        let entry_entry = module_map
            .modules
            .get(entry)
            .ok_or_else(|| ScriptError::Execution(format!("entry '{}' not found", entry)))?;
        let entry_module = v8::Local::new(scope, &entry_entry.module);

        // Precompute referrer-aware resolution for the static resolve
        // callback: source specifiers (e.g. "./utils") are only meaningful
        // relative to the importing module, and the callback receives the
        // raw specifier plus the referrer's `Local<Module>` — nothing else.
        for module_name in &order {
            let info = registry.get(module_name).ok_or_else(|| {
                ScriptError::Execution(format!("module '{}' not found", module_name))
            })?;
            for spec in &info.imports {
                let resolved = registry.resolve(spec, module_name).ok_or_else(|| {
                    ScriptError::Execution(format!(
                        "cannot resolve '{}' from '{}'",
                        spec, module_name
                    ))
                })?;
                module_map
                    .resolutions
                    .insert((module_name.clone(), spec.clone()), resolved);
            }
            let module_entry = module_map.modules.get(module_name).unwrap();
            let hash = v8::Local::new(scope, &module_entry.module).get_identity_hash();
            module_map.by_hash.insert(hash, module_name.clone());
        }

        // Step 3: instantiate with isolate slot (Deno pattern).
        v8::tc_scope!(let tc_scope, scope);
        tc_scope.set_slot(&module_map as *const ModuleMap);

        let instantiate_result = entry_module.instantiate_module2(
            tc_scope,
            Self::module_resolve_callback,
            Self::module_source_callback,
        );

        tc_scope.remove_slot::<*const ModuleMap>();

        if instantiate_result.is_none() {
            return Err(ScriptError::Execution(v8_exception!(
                tc_scope,
                &self.source_maps
            )));
        }

        // Step 4: evaluate the entry module.
        let evaluate_result = entry_module.evaluate(tc_scope);
        if evaluate_result.is_none() {
            return Err(ScriptError::Execution(v8_exception!(
                tc_scope,
                &self.source_maps
            )));
        }
        // Pump the microtask queue so module-level promises and top-level
        // await continuations settle before we touch the exports.
        tc_scope.perform_microtask_checkpoint();

        // Step 5: call entry module's exports.main().
        let namespace = entry_module.get_module_namespace();
        let namespace_obj = v8::Local::<v8::Object>::try_from(namespace)
            .map_err(|_| ScriptError::Runtime("entry module namespace is not an object".into()))?;

        // Cache the namespace for `call_module_export`.
        self.module_namespace = Some(v8::Global::new(tc_scope, namespace_obj));

        let main_name = v8::String::new(tc_scope, "main").unwrap();
        if let Some(main_val) = namespace_obj.get(tc_scope, main_name.into()) {
            if let Ok(main_func) = v8::Local::<v8::Function>::try_from(main_val) {
                let recv: v8::Local<v8::Value> = v8::undefined(tc_scope).into();
                if main_func.call(tc_scope, recv, &[]).is_none() {
                    return Err(ScriptError::Runtime(format!(
                        "main: {}",
                        v8_exception!(tc_scope, &self.source_maps)
                    )));
                }
                tc_scope.perform_microtask_checkpoint();
            }
        }

        // Cache the registry (move Rc, no clone) and entry for hot reload.
        self.registry = Some(registry);
        self.entry = Some(entry.to_string());

        // Evict compiled modules that are no longer in the dependency graph.
        // This prevents Global<Module> handle leaks from stale modules.
        let active: std::collections::HashSet<&str> = order.iter().map(|s| s.as_str()).collect();
        self.compiled_modules
            .retain(|name, _| active.contains(name.as_str()));
        self.source_maps.retain(&active);

        Ok(())
    }

    /// Static resolve callback used by V8 during `Module::instantiate_module2`.
    /// Reads the `ModuleMap` pointer from the isolate slot, then resolves the
    /// raw specifier against the referrer via the precomputed map.
    fn module_resolve_callback<'s>(
        context: v8::Local<'s, v8::Context>,
        specifier: v8::Local<'s, v8::String>,
        _import_attributes: v8::Local<'s, v8::FixedArray>,
        referrer: v8::Local<'s, v8::Module>,
    ) -> Option<v8::Local<'s, v8::Module>> {
        v8::callback_scope!(unsafe scope, context);
        let module_map = unsafe {
            scope
                .get_slot::<*const ModuleMap>()
                .unwrap()
                .as_ref()
                .unwrap()
        };

        let specifier_str = specifier.to_rust_string_lossy(scope);

        // Resolve (importer, specifier) → canonical module; fall back to
        // direct name matches for absolute specifiers.
        let entry = module_map
            .by_hash
            .get(&referrer.get_identity_hash())
            .and_then(|importer| {
                module_map
                    .resolutions
                    .get(&(importer.clone(), specifier_str.clone()))
            })
            .and_then(|canonical| module_map.modules.get(canonical))
            .or_else(|| module_map.modules.get(&specifier_str))
            .or_else(|| {
                let with_js = format!("{}.js", specifier_str);
                module_map.modules.get(&with_js)
            })
            .or_else(|| {
                let with_ts = format!("{}.ts", specifier_str);
                module_map.modules.get(&with_ts)
            });

        match entry {
            Some(e) => {
                let module = v8::Local::new(scope, &e.module);
                Some(module)
            }
            None => {
                let msg = format!("cannot resolve module '{}'", specifier_str);
                let exc = v8::String::new(scope, &msg).unwrap();
                scope.throw_exception(exc.into());
                None
            }
        }
    }

    /// Static source callback for host-defined modules (not used, but required
    /// by `instantiate_module2`).
    fn module_source_callback<'s>(
        _context: v8::Local<'s, v8::Context>,
        _specifier: v8::Local<'s, v8::String>,
        _import_attributes: v8::Local<'s, v8::FixedArray>,
        _referrer: v8::Local<'s, v8::Module>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        None
    }
}

impl HotReloadHandler for V8Runtime {
    fn on_file_changed(&mut self, changed_path: &Path) -> Result<()> {
        self.on_files_changed(std::slice::from_ref(&changed_path.to_path_buf()))
    }

    fn on_files_changed(&mut self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        // Take the cached registry Rc out to avoid double borrow.
        let registry_rc = self
            .registry
            .take()
            .ok_or_else(|| ScriptError::Execution("no cached registry for hot reload".into()))?;
        let entry = self
            .entry
            .clone()
            .ok_or_else(|| ScriptError::Execution("no cached entry for hot reload".into()))?;

        // Regain ownership of the registry. `Rc::try_unwrap` succeeds
        // (only 1 ref after `take()`), avoiding a full clone of all
        // module source strings.
        let mut registry = Rc::try_unwrap(registry_rc).unwrap_or_else(|rc| (*rc).clone());

        // A created file can turn a previously failed resolution into a
        // hit — the memoized probes are no longer trustworthy.
        registry.invalidate_resolution_caches();

        // Process all changed files: update sources, then compute the union
        // of affected modules (changed module + its transitive importers)
        // with a single reverse-dependency pass. This avoids multiple
        // load_module_graph calls when several files change simultaneously.
        // On a read failure, keep the remaining state intact so the runtime
        // keeps running the previously evaluated code.
        let mut changed_modules = Vec::new();
        let mut result: Result<()> = Ok(());
        for path in paths {
            match super::load_script(path) {
                Ok(source) => {
                    // The file watcher reports absolute paths, but the
                    // registry uses relative canonical names; `register`
                    // canonicalizes the fallback.
                    let module_name = registry
                        .find_by_file_path(path)
                        .unwrap_or_else(|| path.to_str().unwrap_or("").to_string());
                    registry.register(&module_name, source);
                    changed_modules.push(module_name);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }

        // Changed modules may import files that were never loaded before —
        // discover and register those now (a no-op when imports are
        // unchanged). Without this, adding an import during an edit fails
        // the reload with "module not found".
        if result.is_ok() {
            for module_name in &changed_modules {
                if let Err(e) = registry.resolve_dependencies(module_name) {
                    result = Err(ScriptError::Execution(e));
                    break;
                }
            }
        }

        let registry_rc = Rc::new(registry);
        if result.is_ok() {
            // V8 modules are one-shot (can't re-instantiate/re-evaluate), so
            // evict the affected set from the cache to force re-compilation;
            // unaffected modules reuse their cached (already evaluated)
            // handles. Then re-instantiate the graph in one pass.
            let all_affected = registry_rc.transitive_importers_many(&changed_modules);
            for name in &all_affected {
                self.compiled_modules.remove(name);
            }
            result = self.load_module_graph(registry_rc.clone(), &entry);
        }

        if result.is_err() {
            // On failure, `load_module_graph` bailed before caching the new
            // registry — restore it here so the next file change retries from
            // the latest on-disk sources instead of failing permanently with
            // "no cached registry".
            self.registry = Some(registry_rc);
            self.entry = Some(entry);
        }
        result
    }
}

/// A compiled module handle cached for incremental hot reload.
///
/// The `source` field enables change detection: if the source matches, the
/// cached `Global<Module>` can be reused without recompilation.
struct CachedModule {
    module: v8::Global<v8::Module>,
    source: String,
}

/// A compiled module handle and its exports namespace.
struct ModuleEntry {
    module: v8::Global<v8::Module>,
    #[allow(dead_code)]
    namespace: Option<v8::Global<v8::Object>>,
}

/// A map of compiled modules, stored temporarily in the isolate slot during
/// `instantiate_module2`. Follows Deno's approach: the static resolve callback
/// retrieves the `*const ModuleMap` from the slot and resolves by name.
struct ModuleMap {
    modules: HashMap<String, ModuleEntry>,
    /// V8 module identity hash → canonical name, so the resolve callback can
    /// recover the referrer's name from the `Local<Module>` it is given.
    by_hash: HashMap<std::num::NonZero<i32>, String>,
    /// (importer canonical name, raw specifier) → resolved canonical name.
    /// Raw specifiers in source text (e.g. `"./utils"`) only make sense
    /// relative to the importing module, so they are pre-resolved when the
    /// graph is built.
    resolutions: HashMap<(String, String), String>,
    #[allow(dead_code)]
    names: Vec<String>,
}

/// State shared between the runtime and its watchdog thread.
///
/// Lock-free on purpose: `arm`/`disarm` run on the script thread at every
/// top-level entry point (i.e. every frame for `on_update`), so they must
/// not take a mutex or signal a condvar.
struct WatchdogShared {
    /// Monotonic reference point for `deadline_ms`.
    epoch: Instant,
    /// Deadline of the currently running execution, in milliseconds since
    /// `epoch`; `0` means disarmed.
    deadline_ms: AtomicU64,
    /// Set when the watchdog thread should exit.
    stop: AtomicBool,
}

/// Terminates runaway script executions. A dedicated thread polls the
/// deadline armed at each top-level entry point; when execution exceeds it,
/// `terminate_execution` is called on the isolate, failing the JS call with
/// an exception (which the entry points surface as a `ScriptError`).
struct ExecutionWatchdog {
    shared: Arc<WatchdogShared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// How often the watchdog thread checks the deadline. Detection latency is
/// at most one interval past the deadline — negligible next to the
/// multi-second timeouts this guards.
const WATCHDOG_POLL_INTERVAL: Duration = Duration::from_millis(5);

impl ExecutionWatchdog {
    fn new(isolate: &v8::OwnedIsolate) -> Self {
        let handle = isolate.thread_safe_handle();
        let shared = Arc::new(WatchdogShared {
            epoch: Instant::now(),
            deadline_ms: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("v8-script-watchdog".into())
            .spawn(move || loop {
                if thread_shared.stop.load(Ordering::Acquire) {
                    break;
                }
                let deadline = thread_shared.deadline_ms.load(Ordering::Acquire);
                if deadline != 0 && thread_shared.epoch.elapsed().as_millis() as u64 >= deadline {
                    handle.terminate_execution();
                    // Disarm so a single overrun terminates only once; the
                    // next top-level entry point re-arms.
                    thread_shared.deadline_ms.store(0, Ordering::Release);
                }
                std::thread::sleep(WATCHDOG_POLL_INTERVAL);
            })
            .ok();
        Self { shared, thread }
    }

    /// Arm the watchdog for one top-level execution. Disarms on drop.
    fn arm(&self, timeout: Duration) -> ArmGuard<'_> {
        let now_ms = self.shared.epoch.elapsed().as_millis() as u64;
        // `max(1)` keeps a zero-length timeout distinguishable from "disarmed".
        let deadline = (now_ms + timeout.as_millis() as u64).max(1);
        self.shared.deadline_ms.store(deadline, Ordering::Release);
        ArmGuard { watchdog: self }
    }

    fn disarm(&self) {
        self.shared.deadline_ms.store(0, Ordering::Release);
    }
}

/// RAII guard that disarms the watchdog when the execution returns.
struct ArmGuard<'a> {
    watchdog: &'a ExecutionWatchdog,
}

impl Drop for ArmGuard<'_> {
    fn drop(&mut self) {
        self.watchdog.disarm();
    }
}

impl Drop for ExecutionWatchdog {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Zero-copy conversion of a V8 typed array to a `HostValue::TypedArrayView`.
/// Falls back to an empty owned `TypedArray` if the backing store is gone.
macro_rules! v8_typed_array_to_host {
    ($value:expr, $scope:expr, $array_ty:ident, $element:ident, $variant:ident) => {
        if let Ok(ta) = v8::Local::<v8::$array_ty>::try_from($value) {
            let byte_length = ta.byte_length();
            if let Some(buf) = ta.buffer($scope) {
                if let Some(ptr) = buf.data() {
                    let offset = ta.byte_offset();
                    return HostValue::TypedArrayView {
                        data: unsafe { (ptr.as_ptr() as *const u8).add(offset) },
                        len: byte_length,
                        element: TypedArrayElement::$element,
                    };
                }
            }
            return HostValue::TypedArray(TypedArrayValue::$variant(Vec::new()));
        }
    };
}

/// Copy a Rust slice into a fresh V8 typed array of the given type.
macro_rules! host_typed_array_to_v8 {
    ($scope:expr, $array_ty:ident, $data:expr, $elem_size:expr) => {{
        let byte_len = $data.len() * $elem_size;
        let buf = v8::ArrayBuffer::new($scope, byte_len);
        if let Some(ptr) = buf.data() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    $data.as_ptr() as *const u8,
                    ptr.as_ptr() as *mut u8,
                    byte_len,
                );
            }
        }
        v8::$array_ty::new($scope, buf, 0, $data.len())
            .unwrap()
            .into()
    }};
}

/// Convert a V8 `Local<Value>` to a `HostValue`.
fn v8_value_to_host(value: v8::Local<v8::Value>, scope: &mut v8::PinScope) -> HostValue {
    if value.is_null_or_undefined() {
        return HostValue::Null;
    }
    if value.is_number() {
        return HostValue::Number(value.number_value(scope).unwrap_or(0.0));
    }
    if value.is_boolean() {
        return HostValue::Bool(value.is_true());
    }
    if value.is_string() {
        return HostValue::String(value.to_rust_string_lossy(scope));
    }

    // ArrayBuffer — zero-copy via BytesView
    if value.is_array_buffer() {
        if let Ok(buf) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
            let len = buf.byte_length();
            if let Some(ptr) = buf.data() {
                return HostValue::BytesView {
                    data: ptr.as_ptr() as *const u8,
                    len,
                };
            }
            return HostValue::ArrayBuffer(Vec::new());
        }
    }

    // TypedArray — zero-copy via TypedArrayView (Uint8 uses BytesView for as_bytes_view compat)
    if value.is_typed_array() {
        // Uint8Array — zero-copy via BytesView
        if let Ok(ta) = v8::Local::<v8::Uint8Array>::try_from(value) {
            let byte_length = ta.byte_length();
            if let Some(buf) = ta.buffer(scope) {
                if let Some(ptr) = buf.data() {
                    let offset = ta.byte_offset();
                    return HostValue::BytesView {
                        data: unsafe { (ptr.as_ptr() as *const u8).add(offset) },
                        len: byte_length,
                    };
                }
            }
            return HostValue::TypedArray(TypedArrayValue::Uint8(Vec::new()));
        }
        // Remaining typed array types — zero-copy via TypedArrayView.
        v8_typed_array_to_host!(value, scope, Float32Array, Float32, Float32);
        v8_typed_array_to_host!(value, scope, Float64Array, Float64, Float64);
        v8_typed_array_to_host!(value, scope, Int8Array, Int8, Int8);
        v8_typed_array_to_host!(value, scope, Uint16Array, Uint16, Uint16);
        v8_typed_array_to_host!(value, scope, Int16Array, Int16, Int16);
        v8_typed_array_to_host!(value, scope, Uint32Array, Uint32, Uint32);
        v8_typed_array_to_host!(value, scope, Int32Array, Int32, Int32);
    }

    // Array
    if value.is_array() {
        if let Ok(arr) = v8::Local::<v8::Array>::try_from(value) {
            let len = arr.length() as usize;
            let mut items = Vec::with_capacity(len);
            for i in 0..len {
                if let Some(item) = arr.get_index(scope, i as u32) {
                    items.push(v8_value_to_host(item, scope));
                }
            }
            return HostValue::Array(items);
        }
    }

    // Object
    if value.is_object() {
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(value) {
            let mut map = std::collections::HashMap::new();
            let names = obj.get_own_property_names(scope, Default::default());
            if let Some(names) = names {
                let len = names.length() as usize;
                for i in 0..len {
                    if let Some(key_val) = names.get_index(scope, i as u32) {
                        let key_str = key_val.to_rust_string_lossy(scope);
                        let key = v8::String::new(scope, &key_str).unwrap();
                        if let Some(val) = obj.get(scope, key.into()) {
                            map.insert(key_str, v8_value_to_host(val, scope));
                        }
                    }
                }
            }
            return HostValue::Object(map);
        }
    }

    // Fallback: stringify.
    HostValue::String(value.to_rust_string_lossy(scope))
}

/// Convert a `HostValue` to a V8 `Local<Value>`.
///
/// Takes a reference so callers don't pay a deep clone per argument; only
/// the single copy into the V8 heap remains.
fn host_to_v8_value<'s>(
    value: &HostValue,
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Value> {
    match value {
        HostValue::Null => v8::null(scope).into(),
        HostValue::Bool(b) => v8::Boolean::new(scope, *b).into(),
        HostValue::Number(n) => v8::Number::new(scope, *n).into(),
        HostValue::String(s) => v8::String::new(scope, s).unwrap().into(),
        HostValue::ArrayBuffer(data) => {
            let buf = v8::ArrayBuffer::new(scope, data.len());
            if let Some(ptr) = buf.data() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data.as_ptr(),
                        ptr.as_ptr() as *mut u8,
                        data.len(),
                    );
                }
            }
            buf.into()
        }
        HostValue::BytesView { data, len } => {
            let buf = v8::ArrayBuffer::new(scope, *len);
            if let Some(ptr) = buf.data() {
                unsafe {
                    std::ptr::copy_nonoverlapping(*data, ptr.as_ptr() as *mut u8, *len);
                }
            }
            buf.into()
        }
        HostValue::TypedArrayView { data, len, element } => {
            // TypedArrayView is a zero-copy view into V8's own backing store.
            // When returned back to V8, we copy the data into a new ArrayBuffer
            // and wrap it in the appropriate typed array.
            let buf = v8::ArrayBuffer::new(scope, *len);
            if let Some(ptr) = buf.data() {
                unsafe {
                    std::ptr::copy_nonoverlapping(*data, ptr.as_ptr() as *mut u8, *len);
                }
            }
            let count = *len
                / match element {
                    TypedArrayElement::Uint8 | TypedArrayElement::Int8 => 1,
                    TypedArrayElement::Uint16 | TypedArrayElement::Int16 => 2,
                    TypedArrayElement::Uint32
                    | TypedArrayElement::Int32
                    | TypedArrayElement::Float32 => 4,
                    TypedArrayElement::Float64 => 8,
                };
            match element {
                TypedArrayElement::Uint8 => {
                    v8::Uint8Array::new(scope, buf, 0, count).unwrap().into()
                }
                TypedArrayElement::Int8 => v8::Int8Array::new(scope, buf, 0, count).unwrap().into(),
                TypedArrayElement::Uint16 => {
                    v8::Uint16Array::new(scope, buf, 0, count).unwrap().into()
                }
                TypedArrayElement::Int16 => {
                    v8::Int16Array::new(scope, buf, 0, count).unwrap().into()
                }
                TypedArrayElement::Uint32 => {
                    v8::Uint32Array::new(scope, buf, 0, count).unwrap().into()
                }
                TypedArrayElement::Int32 => {
                    v8::Int32Array::new(scope, buf, 0, count).unwrap().into()
                }
                TypedArrayElement::Float32 => {
                    v8::Float32Array::new(scope, buf, 0, count).unwrap().into()
                }
                TypedArrayElement::Float64 => {
                    v8::Float64Array::new(scope, buf, 0, count).unwrap().into()
                }
            }
        }
        HostValue::Object(map) => {
            let obj = v8::Object::new(scope);
            for (k, v) in map {
                let key = v8::String::new(scope, k).unwrap();
                let val = host_to_v8_value(v, scope);
                obj.set(scope, key.into(), val);
            }
            obj.into()
        }
        HostValue::Array(items) => {
            let len = items.len() as u32;
            let arr = v8::Array::new(scope, len as i32);
            for (i, item) in items.iter().enumerate() {
                let val = host_to_v8_value(item, scope);
                arr.set_index(scope, i as u32, val);
            }
            arr.into()
        }
        HostValue::TypedArray(ta) => {
            let val: v8::Local<'s, v8::Value> = match ta {
                TypedArrayValue::Uint8(data) => host_typed_array_to_v8!(scope, Uint8Array, data, 1),
                TypedArrayValue::Int8(data) => host_typed_array_to_v8!(scope, Int8Array, data, 1),
                TypedArrayValue::Uint16(data) => {
                    host_typed_array_to_v8!(scope, Uint16Array, data, 2)
                }
                TypedArrayValue::Int16(data) => {
                    host_typed_array_to_v8!(scope, Int16Array, data, 2)
                }
                TypedArrayValue::Uint32(data) => {
                    host_typed_array_to_v8!(scope, Uint32Array, data, 4)
                }
                TypedArrayValue::Int32(data) => {
                    host_typed_array_to_v8!(scope, Int32Array, data, 4)
                }
                TypedArrayValue::Float32(data) => {
                    host_typed_array_to_v8!(scope, Float32Array, data, 4)
                }
                TypedArrayValue::Float64(data) => {
                    host_typed_array_to_v8!(scope, Float64Array, data, 8)
                }
            };
            val
        }
    }
}

/// Throw a JS exception carrying `msg`.
///
/// `v8::String::new` only fails when the message exceeds V8's maximum string
/// length; fall back to a static message in that case instead of panicking
/// inside a callback.
fn throw_host_error(scope: &mut v8::PinScope, msg: &str) {
    let exc = v8::String::new(scope, msg).or_else(|| v8::String::new(scope, "host function error"));
    if let Some(exc) = exc {
        scope.throw_exception(exc.into());
    }
}

/// Extract a human-readable exception (message + location + stack frames) from
/// a V8 `TryCatch` scope, remapping locations through the source map cache
/// when available. Used as a macro because the `tc` scope type is an opaque
/// pinned projection that is awkward to name generically.
macro_rules! v8_exception {
    ($tc:expr, $maps:expr) => {{
        let tc = $tc;
        let maps: &crate::script::source_map::SourceMapCache = $maps;
        if !tc.has_caught() {
            String::from("unknown error")
        } else {
            let mut out = String::new();
            if let Some(exc) = tc.exception() {
                out.push_str(&exc.to_rust_string_lossy(tc));
            }
            if let Some(msg) = tc.message() {
                let mut file = String::new();
                if let Some(res) = msg.get_script_resource_name(tc) {
                    let r = res.to_rust_string_lossy(tc);
                    if !r.is_empty() && r != "undefined" {
                        file = r;
                    }
                }
                let mut line = msg.get_line_number(tc).map(|l| l as u32);
                // Best-effort remap to the original TypeScript location.
                if let Some(l) = line {
                    if let Some((src, rl, _)) = maps.remap(&file, l, 1) {
                        file = src;
                        line = Some(rl);
                    }
                }
                let mut loc = file;
                if let Some(line) = line {
                    if !loc.is_empty() {
                        loc.push(':');
                    }
                    loc.push_str(&line.to_string());
                }
                if !loc.is_empty() {
                    out.push_str(&format!("\n  at {}", loc));
                }
                if let Some(st) = msg.get_stack_trace(tc) {
                    let count = st.get_frame_count().min(10);
                    for i in 0..count {
                        if let Some(frame) = st.get_frame(tc, i) {
                            let mut f = String::new();
                            let has_name = if let Some(name) = frame.get_function_name(tc) {
                                let n = name.to_rust_string_lossy(tc);
                                if !n.is_empty() {
                                    f.push_str(&n);
                                    f.push_str(" (");
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            let mut script_name = frame
                                .get_script_name_or_source_url(tc)
                                .map(|s| s.to_rust_string_lossy(tc))
                                .unwrap_or_default();
                            let mut line = frame.get_line_number() as u32;
                            let mut col = frame.get_column() as u32;
                            // Best-effort remap to the original TS location.
                            if let Some((src, rl, rc)) = maps.remap(&script_name, line, col) {
                                script_name = src;
                                line = rl;
                                col = rc;
                            }
                            f.push_str(&script_name);
                            f.push_str(&format!(":{}:{}", line, col));
                            if has_name {
                                f.push(')');
                            }
                            if !f.is_empty() {
                                out.push_str(&format!("\n    at {}", f));
                            }
                        }
                    }
                }
            }
            out
        }
    }};
}
use v8_exception;

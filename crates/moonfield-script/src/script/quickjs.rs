//! QuickJS backend for the scripting runtime.

use super::{
    HostValue, HotReloadHandler, ModuleRegistry, Result, ScriptApi, ScriptError, ScriptRuntime,
    TypedArrayElement, TypedArrayValue, DEFAULT_EXECUTION_TIMEOUT, DEFAULT_MAX_HEAP_BYTES,
};
use moonfield_log::{error, info, warn};
use rquickjs::function::{Args, Func, Rest};
use rquickjs::{CaughtError, Context, Ctx, Function, IntoJs, Object, Runtime, TypedArray, Value};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// RAII guard that clears the execution deadline when a call returns.
struct DeadlineGuard(Rc<RefCell<Option<Instant>>>);

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        *self.0.borrow_mut() = None;
    }
}

/// JavaScript shim that wires a `console` object to the `__mf_log` host
/// function registered below. Stringifying via `String(...)` matches browser
/// `console.log` semantics.
const CONSOLE_SHIM: &str = r#"
globalThis.console = {
  log:   function() { __mf_log(0, Array.prototype.map.call(arguments, String).join(" ")); },
  info:  function() { __mf_log(0, Array.prototype.map.call(arguments, String).join(" ")); },
  warn:  function() { __mf_log(1, Array.prototype.map.call(arguments, String).join(" ")); },
  error: function() { __mf_log(2, Array.prototype.map.call(arguments, String).join(" ")); }
};
"#;

/// A script runtime backed by QuickJS.
///
/// Module system: [`load_module_graph`] bundles the whole module graph into a
/// single self-contained script (per-module factories plus a tiny
/// CommonJS-style `require`) and evaluates it in one pass. The entry module's
/// exports are exposed as `globalThis.__mfEntryExports`.
///
/// Hot reload: re-evaluates the full bundle on file change (QuickJS has no
/// incremental compiled-module cache like V8). Runtime JS state is reset on
/// every reload.
pub struct QuickJsRuntime {
    runtime: Runtime,
    context: Context,
    api: ScriptApi,
    /// Cached registry for hot reload (populated by `load_module_graph`).
    registry: Option<Rc<ModuleRegistry>>,
    /// Cached entry point for hot reload.
    entry: Option<String>,
    /// Deadline of the currently running execution, checked by the engine's
    /// interrupt handler; `None` when idle. Shared with the handler closure.
    deadline: Rc<RefCell<Option<Instant>>>,
    /// Per-execution time limit armed at every top-level entry point.
    execution_timeout: Duration,
    /// Last time `run_gc` ran; `gc_step` throttles full collections.
    last_gc: Instant,
}

impl ScriptRuntime for QuickJsRuntime {
    fn new(api: ScriptApi) -> Result<Self> {
        Self::new_with_memory_limit(api, DEFAULT_MAX_HEAP_BYTES)
    }

    fn load(&mut self, _name: &str, source: &str) -> Result<()> {
        let _deadline_guard = self.arm();
        self.context.with(
            |ctx| match CaughtError::catch(&ctx, ctx.eval::<(), _>(source)) {
                Ok(()) => Ok(()),
                Err(ce) => Err(ScriptError::Execution(format_caught_error(ce))),
            },
        )?;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.context = Context::full(&self.runtime)
            .map_err(|e| ScriptError::Runtime(format!("failed to recreate context: {:?}", e)))?;
        self.register_api()
    }

    fn call(&mut self, function: &str) -> Result<()> {
        let _deadline_guard = self.arm();
        self.context.with(|ctx| {
            let func = ctx
                .globals()
                .get::<_, Function>(function)
                .map_err(|_| ScriptError::Runtime(format!("function '{}' not found", function)))?;
            match CaughtError::catch(&ctx, func.call::<(), Value>(())) {
                Ok(_) => Ok(()),
                Err(ce) => Err(ScriptError::Runtime(format!(
                    "call '{}': {}",
                    function,
                    format_caught_error(ce)
                ))),
            }
        })?;
        Ok(())
    }

    fn call_with_args(&mut self, function: &str, args: &[HostValue]) -> Result<HostValue> {
        let _deadline_guard = self.arm();
        self.context.with(|ctx| {
            let func = ctx
                .globals()
                .get::<_, Function>(function)
                .map_err(|_| ScriptError::Runtime(format!("function '{}' not found", function)))?;
            let js_args = build_args(&ctx, args)?;
            match CaughtError::catch(&ctx, func.call_arg::<Value>(js_args)) {
                Ok(val) => Ok(quickjs_value_to_host(&val)),
                Err(ce) => Err(ScriptError::Runtime(format!(
                    "call '{}': {}",
                    function,
                    format_caught_error(ce)
                ))),
            }
        })
    }

    fn gc_step(&mut self) {
        // run_gc is a full mark-and-sweep pass — far too expensive to run
        // every frame. QuickJS's allocator already collects when the GC
        // threshold is exceeded, so an occasional nudge is enough.
        if self.last_gc.elapsed() >= Duration::from_millis(100) {
            self.runtime.run_gc();
            self.last_gc = Instant::now();
        }
    }

    fn has_function(&mut self, name: &str) -> bool {
        self.context.with(|ctx| {
            let globals = ctx.globals();
            // Check the entry module's exports first, then globals.
            let in_exports = globals
                .get::<_, Object>("__mfEntryExports")
                .ok()
                .and_then(|exports| exports.get::<_, Value>(name).ok())
                .is_some_and(|v| v.is_function());
            in_exports || globals.get::<_, Value>(name).is_ok_and(|v| v.is_function())
        })
    }

    fn call_module_export(&mut self, function: &str, args: &[HostValue]) -> Result<HostValue> {
        self.call_module_export_impl(function, args, true)
            .map(|v| v.unwrap_or(HostValue::Null))
    }

    fn call_module_export_unit(&mut self, function: &str, args: &[HostValue]) -> Result<()> {
        self.call_module_export_impl(function, args, false)
            .map(|_| ())
    }

    fn load_module_graph(&mut self, registry: Rc<ModuleRegistry>, entry: &str) -> Result<()> {
        let _deadline_guard = self.arm();
        let bundle = build_module_bundle(&registry, entry)?;
        self.context.with(
            |ctx| match CaughtError::catch(&ctx, ctx.eval::<(), _>(bundle)) {
                Ok(()) => Ok(()),
                Err(ce) => Err(ScriptError::Execution(format_caught_error(ce))),
            },
        )?;

        // Evaluate the entry module, expose its exports for
        // `call_module_export`, and run `main()` if present.
        let entry_json =
            serde_json::to_string(entry).map_err(|e| ScriptError::Execution(e.to_string()))?;
        let bootstrap = format!(
            "globalThis.__mfEntryExports = __mfRequire({});\n\
             if (typeof __mfEntryExports.main === 'function') {{ __mfEntryExports.main(); }}",
            entry_json
        );
        self.context.with(
            |ctx| match CaughtError::catch(&ctx, ctx.eval::<(), _>(bootstrap)) {
                Ok(()) => Ok(()),
                Err(ce) => Err(ScriptError::Runtime(format_caught_error(ce))),
            },
        )?;

        self.registry = Some(registry);
        self.entry = Some(entry.to_string());
        Ok(())
    }
}

impl QuickJsRuntime {
    /// Create a runtime with an explicit memory limit (in bytes).
    ///
    /// [`ScriptRuntime::new`] uses [`DEFAULT_MAX_HEAP_BYTES`]. The GC
    /// threshold stays at 4 MB so collection kicks in early.
    pub fn new_with_memory_limit(api: ScriptApi, max_bytes: usize) -> Result<Self> {
        let runtime = Runtime::new()
            .map_err(|e| ScriptError::BackendNotAvailable(format!("quickjs: {:?}", e)))?;
        // Give QuickJS a generous stack limit so that host functions (which may
        // call into Vulkan drivers) do not overflow the JS engine's C stack.
        runtime.set_max_stack_size(8 * 1024 * 1024);
        // Cap heap growth and trigger GC proactively.
        runtime.set_memory_limit(max_bytes);
        runtime.set_gc_threshold(4 * 1024 * 1024);

        // Runaway-execution guard: the engine calls this handler regularly;
        // returning true raises an uncatchable exception inside the JS call.
        let deadline: Rc<RefCell<Option<Instant>>> = Rc::new(RefCell::new(None));
        let handler_deadline = Rc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            handler_deadline
                .borrow()
                .map(|dl| Instant::now() >= dl)
                .unwrap_or(false)
        })));

        let context = Context::full(&runtime)
            .map_err(|e| ScriptError::BackendNotAvailable(format!("quickjs context: {:?}", e)))?;

        let mut rt = Self {
            runtime,
            context,
            api,
            registry: None,
            entry: None,
            deadline,
            execution_timeout: DEFAULT_EXECUTION_TIMEOUT,
            last_gc: Instant::now(),
        };
        rt.register_api()?;
        Ok(rt)
    }

    /// Set the per-execution time limit for top-level script calls.
    ///
    /// A call that runs longer than this (e.g. an infinite loop) is
    /// interrupted by the engine's interrupt handler and returns an error;
    /// the runtime stays usable for subsequent calls.
    pub fn set_execution_timeout(&mut self, timeout: Duration) {
        self.execution_timeout = timeout;
    }

    /// Arm the execution deadline for one top-level call. Cleared on drop.
    fn arm(&self) -> DeadlineGuard {
        *self.deadline.borrow_mut() = Some(Instant::now() + self.execution_timeout);
        DeadlineGuard(Rc::clone(&self.deadline))
    }

    /// Shared implementation of `call_module_export` / `call_module_export_unit`.
    /// When `marshal_result` is false the return value is discarded without
    /// converting it to a `HostValue`.
    fn call_module_export_impl(
        &mut self,
        function: &str,
        args: &[HostValue],
        marshal_result: bool,
    ) -> Result<Option<HostValue>> {
        let _deadline_guard = self.arm();
        self.context.with(|ctx| {
            let exports = ctx
                .globals()
                .get::<_, Object>("__mfEntryExports")
                .map_err(|_| ScriptError::Runtime("no module namespace loaded".into()))?;
            let func = exports
                .get::<_, Function>(function)
                .map_err(|_| ScriptError::Runtime(format!("export '{}' not found", function)))?;
            let js_args = build_args(&ctx, args)?;
            match CaughtError::catch(&ctx, func.call_arg::<Value>(js_args)) {
                Ok(val) => Ok(marshal_result.then(|| quickjs_value_to_host(&val))),
                Err(ce) => Err(ScriptError::Runtime(format!(
                    "module export '{}': {}",
                    function,
                    format_caught_error(ce)
                ))),
            }
        })
    }

    fn register_api(&mut self) -> Result<()> {
        self.context
            .with(|ctx| {
                let global = ctx.globals();
                for entry in self.api.iter() {
                    let (name, func) = (entry.0, entry.1.clone());
                    // Use a non-capturing wrapper to avoid borrow issues with `name`.
                    let wrapper = ApiFuncWrapper { name, func };
                    global.set(
                        name,
                        Func::from(
                            move |ctx: Ctx, args: Rest<Value>| -> rquickjs::Result<HostValueJs> {
                                let mut host_args: Vec<HostValue> =
                                    Vec::with_capacity(args.0.len());
                                for arg in args.0.iter() {
                                    host_args.push(quickjs_value_to_host(arg));
                                }
                                // A panicking host function must not unwind
                                // through the QuickJS C stack — catch it and
                                // surface it as a JS exception instead.
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        (wrapper.func)(&host_args)
                                    }));
                                match result {
                                    Ok(Ok(ret)) => Ok(HostValueJs(ret)),
                                    Ok(Err(e)) => Err(rquickjs::Exception::throw_message(
                                        &ctx,
                                        &format!("{}: {}", wrapper.name, e),
                                    )),
                                    Err(payload) => Err(rquickjs::Exception::throw_message(
                                        &ctx,
                                        &format!(
                                            "{} panicked: {}",
                                            wrapper.name,
                                            super::panic_payload_message(&payload)
                                        ),
                                    )),
                                }
                            },
                        ),
                    )?;
                }
                // Host sink for console output: (level, message).
                global.set(
                    "__mf_log",
                    Func::from(|level: i32, msg: String| match level {
                        0 => info!("{}", msg),
                        1 => warn!("{}", msg),
                        _ => error!("{}", msg),
                    }),
                )?;
                ctx.eval::<(), _>(CONSOLE_SHIM)?;
                Ok(())
            })
            .map_err(|e: rquickjs::Error| ScriptError::Runtime(format!("{:?}", e)))?;
        Ok(())
    }
}

/// Helper to hold a (name, func) pair without lifetime issues.
struct ApiFuncWrapper {
    name: &'static str,
    func: super::HostFn,
}

impl HotReloadHandler for QuickJsRuntime {
    fn on_file_changed(&mut self, changed_path: &Path) -> Result<()> {
        self.on_files_changed(std::slice::from_ref(&changed_path.to_path_buf()))
    }

    fn on_files_changed(&mut self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let registry_rc = self
            .registry
            .take()
            .ok_or_else(|| ScriptError::Execution("no cached registry for hot reload".into()))?;
        let entry = self
            .entry
            .clone()
            .ok_or_else(|| ScriptError::Execution("no cached entry for hot reload".into()))?;
        let mut registry = Rc::try_unwrap(registry_rc).unwrap_or_else(|rc| (*rc).clone());

        // QuickJS re-evaluates the whole bundle (no incremental compiled-module
        // cache like V8): update all changed sources, then re-run the graph.
        let mut result: Result<()> = Ok(());
        for path in paths {
            match super::load_script(path) {
                Ok(source) => {
                    let module_name = registry
                        .find_by_file_path(path)
                        .unwrap_or_else(|| path.to_str().unwrap_or("").to_string());
                    registry.register(&module_name, source);
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }

        let registry_rc = Rc::new(registry);
        if result.is_ok() {
            result = self.load_module_graph(registry_rc.clone(), &entry);
        }
        if result.is_err() {
            // Restore so the next file change retries instead of getting
            // permanently stuck on "no cached registry".
            self.registry = Some(registry_rc);
            self.entry = Some(entry);
        }
        result
    }
}

/// Build a self-contained JS bundle defining every module factory plus a tiny
/// CommonJS-style `require`, without evaluating any module yet.
///
/// Layout: a `__resolveMap` (importer → specifier → canonical name) baked in
/// as JSON, one `__factories[name]` function per module wrapping its swc
/// CJS-transformed source, and a `__requireImpl` exposed as
/// `globalThis.__mfRequire`.
fn build_module_bundle(registry: &ModuleRegistry, entry: &str) -> Result<String> {
    let order = registry
        .order_dependencies(entry)
        .map_err(ScriptError::Execution)?;

    // importer canonical name -> { specifier -> resolved canonical name }
    let mut resolve_map = serde_json::Map::new();
    for name in &order {
        let info = registry
            .get(name)
            .ok_or_else(|| ScriptError::Execution(format!("module '{}' not found", name)))?;
        let mut deps = serde_json::Map::new();
        for spec in &info.imports {
            let resolved = registry
                .resolve(spec, name)
                .or_else(|| registry.resolve_full(spec, name).map(|(c, _)| c))
                .ok_or_else(|| {
                    ScriptError::Execution(format!("cannot resolve '{}' from '{}'", spec, name))
                })?;
            deps.insert(spec.clone(), serde_json::Value::String(resolved));
        }
        resolve_map.insert(name.clone(), serde_json::Value::Object(deps));
    }

    let mut bundle = String::from(
        "(function(){\n'use strict';\nvar __cache = {};\nvar __factories = {};\nvar __resolveMap = ",
    );
    bundle.push_str(
        &serde_json::to_string(&serde_json::Value::Object(resolve_map))
            .map_err(|e| ScriptError::Execution(e.to_string()))?,
    );
    bundle.push_str(";\n");

    for name in &order {
        let info = registry
            .get(name)
            .ok_or_else(|| ScriptError::Execution(format!("module '{}' not found", name)))?;
        let name_json =
            serde_json::to_string(name).map_err(|e| ScriptError::Execution(e.to_string()))?;
        bundle.push_str("__factories[");
        bundle.push_str(&name_json);
        bundle.push_str(
            "] = function(module, exports) {\n\
             var __require = function(s) { return __requireImpl((__resolveMap[",
        );
        bundle.push_str(&name_json);
        bundle.push_str("] || {})[s] || s); };\n");
        bundle.push_str(&info.cjs_source);
        bundle.push_str("\n};\n");
    }

    bundle.push_str(
        "function __requireImpl(name) {\n\
           if (__cache[name]) return __cache[name].exports;\n\
           var factory = __factories[name];\n\
           if (!factory) throw new Error('module not found: ' + name);\n\
           var module = { exports: {} };\n\
           __cache[name] = module;\n\
           factory(module, module.exports);\n\
           return module.exports;\n\
         }\n\
         globalThis.__mfRequire = __requireImpl;\n\
         })();\n",
    );
    Ok(bundle)
}

/// Owned wrapper that converts a host function's return value into a JS
/// value via [`IntoJs`]. The framework performs the conversion with the
/// correct `Ctx`, which keeps the host-function closures free of borrowed
/// lifetimes in their return types.
struct HostValueJs(HostValue);

impl<'js> IntoJs<'js> for HostValueJs {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        host_value_to_js(ctx, &self.0)
    }
}

/// Build a QuickJS argument list from marshaled host values.
fn build_args<'js>(ctx: &Ctx<'js>, args: &[HostValue]) -> Result<Args<'js>> {
    let mut js_args = Args::new_unsized(ctx.clone());
    for a in args {
        let v = host_value_to_js(ctx, a)
            .map_err(|e| ScriptError::Runtime(format!("marshal arg: {:?}", e)))?;
        js_args
            .push_arg(v)
            .map_err(|e| ScriptError::Runtime(format!("marshal arg: {:?}", e)))?;
    }
    Ok(js_args)
}

/// Copy a raw byte view into a QuickJS typed array of the given element type.
macro_rules! typed_array_view_to_js {
    ($ctx:expr, $data:expr, $len:expr, $ty:ty, $size:expr) => {{
        let slice = unsafe { std::slice::from_raw_parts(*$data as *const $ty, *$len / $size) };
        TypedArray::<$ty>::new_copy($ctx.clone(), slice).map(|ta| ta.into_value())
    }};
}

/// Convert a `HostValue` to a QuickJS `Value`, preserving types faithfully
/// (NaN/Infinity, typed arrays) — unlike the old JSON-string round-trip,
/// which mangled non-finite floats and degraded typed arrays to plain arrays.
fn host_value_to_js<'js>(ctx: &Ctx<'js>, value: &HostValue) -> rquickjs::Result<Value<'js>> {
    Ok(match value {
        HostValue::Null => Value::new_null(ctx.clone()),
        HostValue::Bool(b) => (*b).into_js(ctx)?,
        HostValue::Number(n) => (*n).into_js(ctx)?,
        HostValue::String(s) => s.as_str().into_js(ctx)?,
        HostValue::Array(items) => {
            let arr = rquickjs::Array::new(ctx.clone())?;
            for (i, item) in items.iter().enumerate() {
                arr.set(i, host_value_to_js(ctx, item)?)?;
            }
            arr.into_value()
        }
        HostValue::Object(map) => {
            let obj = Object::new(ctx.clone())?;
            for (k, v) in map {
                obj.set(k.as_str(), host_value_to_js(ctx, v)?)?;
            }
            obj.into_value()
        }
        HostValue::ArrayBuffer(buf) => TypedArray::<u8>::new_copy(ctx.clone(), buf)?.into_value(),
        HostValue::BytesView { data, len } => {
            let slice = unsafe { std::slice::from_raw_parts(*data, *len) };
            TypedArray::<u8>::new_copy(ctx.clone(), slice)?.into_value()
        }
        HostValue::TypedArrayView { data, len, element } => match element {
            TypedArrayElement::Uint8 => typed_array_view_to_js!(ctx, data, len, u8, 1)?,
            TypedArrayElement::Int8 => typed_array_view_to_js!(ctx, data, len, i8, 1)?,
            TypedArrayElement::Uint16 => typed_array_view_to_js!(ctx, data, len, u16, 2)?,
            TypedArrayElement::Int16 => typed_array_view_to_js!(ctx, data, len, i16, 2)?,
            TypedArrayElement::Uint32 => typed_array_view_to_js!(ctx, data, len, u32, 4)?,
            TypedArrayElement::Int32 => typed_array_view_to_js!(ctx, data, len, i32, 4)?,
            TypedArrayElement::Float32 => typed_array_view_to_js!(ctx, data, len, f32, 4)?,
            TypedArrayElement::Float64 => typed_array_view_to_js!(ctx, data, len, f64, 8)?,
        },
        HostValue::TypedArray(ta) => match ta {
            TypedArrayValue::Uint8(v) => TypedArray::<u8>::new_copy(ctx.clone(), v)?.into_value(),
            TypedArrayValue::Int8(v) => TypedArray::<i8>::new_copy(ctx.clone(), v)?.into_value(),
            TypedArrayValue::Uint16(v) => TypedArray::<u16>::new_copy(ctx.clone(), v)?.into_value(),
            TypedArrayValue::Int16(v) => TypedArray::<i16>::new_copy(ctx.clone(), v)?.into_value(),
            TypedArrayValue::Uint32(v) => TypedArray::<u32>::new_copy(ctx.clone(), v)?.into_value(),
            TypedArrayValue::Int32(v) => TypedArray::<i32>::new_copy(ctx.clone(), v)?.into_value(),
            TypedArrayValue::Float32(v) => {
                TypedArray::<f32>::new_copy(ctx.clone(), v)?.into_value()
            }
            TypedArrayValue::Float64(v) => {
                TypedArray::<f64>::new_copy(ctx.clone(), v)?.into_value()
            }
        },
    })
}

/// Convert a QuickJS `Value` to a `HostValue`.
fn quickjs_value_to_host(value: &Value) -> HostValue {
    if value.is_undefined() || value.is_null() {
        return HostValue::Null;
    }
    if let Some(b) = value.as_bool() {
        return HostValue::Bool(b);
    }
    if let Some(n) = value.as_float() {
        return HostValue::Number(n);
    }
    if let Some(n) = value.as_int() {
        return HostValue::Number(n as f64);
    }
    if let Some(s) = value.as_string() {
        if let Ok(s) = s.to_string() {
            return HostValue::String(s);
        }
    }
    if let Some(arr) = value.as_array() {
        let mut items = Vec::new();
        for item in arr.iter().flatten() {
            items.push(quickjs_value_to_host(&item));
        }
        return HostValue::Array(items);
    }
    if let Some(obj) = value.as_object() {
        let mut map = std::collections::HashMap::new();
        for key in obj.keys::<String>().flatten() {
            if let Ok(val) = obj.get::<_, Value>(&key) {
                map.insert(key, quickjs_value_to_host(&val));
            }
        }
        return HostValue::Object(map);
    }
    // Fallback: stringify
    if let Some(s) = value.as_string() {
        if let Ok(s) = s.to_string() {
            return HostValue::String(s);
        }
    }
    HostValue::String(format!("{:?}", value))
}

/// Turn a caught QuickJS error into a human-readable string with message,
/// location, and (if present) stack trace.
fn format_caught_error<'js>(ce: CaughtError<'js>) -> String {
    match ce {
        CaughtError::Exception(e) => {
            let mut s = e.message().unwrap_or_else(|| "exception".to_string());
            if let (Some(file), Some(line)) = (e.file(), e.line()) {
                s.push_str(&format!("\n  at {}:{}", file, line));
            }
            if let Some(stack) = e.stack() {
                let stack = stack.trim();
                if !stack.is_empty() {
                    s.push_str(&format!("\n{}", stack));
                }
            }
            s
        }
        CaughtError::Value(v) => format!("thrown value: {:?}", v),
        CaughtError::Error(e) => format!("{:?}", e),
    }
}

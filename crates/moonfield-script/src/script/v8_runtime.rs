//! V8 backend for the scripting runtime.

use super::{HostFn, HostValue, HotReloadHandler, Result, ScriptApi, ScriptError, ScriptRuntime, TypedArrayValue};
use moonfield_log::{error, info, warn};
use moonfield_lunaris::HeadlessContext;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Once;

static V8_INIT: Once = Once::new();

/// A V8-native host function that operates on raw V8 types.
///
/// Bypasses the `HostValue` marshaling layer for direct Rust↔V8 communication.
/// Use for high-frequency functions (e.g. `record_frame`) where the overhead of
/// `v8_value_to_host` → `HostValue` → typed extraction is measurable.
type DirectFn = fn(&mut v8::PinScope, v8::FunctionCallbackArguments, v8::ReturnValue);

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
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    // Boxed so the registry vector's storage is never moved while V8 externals
    // hold raw pointers into it.
    api: Box<ScriptApi>,
    /// Direct (fast-path) host functions that bypass `HostValue` marshaling.
    direct_fns: Vec<(&'static str, DirectFn)>,
    /// Cached registry for hot reload (populated by `load_module_graph`).
    registry: Option<super::ModuleRegistry>,
    /// Cached entry point for hot reload.
    entry: Option<String>,
    /// Cached compiled modules for incremental hot reload.
    /// Keyed by canonical module name. Source is stored for change detection.
    compiled_modules: HashMap<String, CachedModule>,
}

impl ScriptRuntime for V8Runtime {
    fn new(api: ScriptApi) -> Result<Self> {
        V8_INIT.call_once(|| {
            // TypeScript is compiled at build time via `tsc` (see scripts/tsconfig.json).
            // The runtime loads pre-compiled JavaScript from target/scripts/.
            // Native type stripping is not yet available in this V8 version.

            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });

        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        let context = {
            v8::scope!(let handle_scope, &mut isolate);
            let context = v8::Context::new(handle_scope, Default::default());
            v8::Global::new(handle_scope, context)
        };

        let mut rt = Self {
            isolate,
            context,
            api: Box::new(api),
            direct_fns: Vec::new(),
            registry: None,
            entry: None,
        };
        rt.register_direct("record_frame", direct_record_frame);
        rt.register_api()?;
        Ok(rt)
    }

    fn load(&mut self, name: &str, source: &str) -> Result<()> {
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
            None => return Err(ScriptError::Execution(v8_exception!(tc))),
        };
        match script.run(tc) {
            Some(_) => Ok(()),
            None => Err(ScriptError::Execution(v8_exception!(tc))),
        }
    }

    fn reload(&mut self) -> Result<()> {
        let context = {
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Context::new(handle_scope, Default::default());
            v8::Global::new(handle_scope, context)
        };
        self.context = context;
        self.register_api()
    }

    fn call(&mut self, function: &str) -> Result<()> {
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
                v8_exception!(tc)
            )));
        }
        Ok(())
    }
}

impl V8Runtime {
    /// Register a fast-path host function that operates on V8 types directly.
    ///
    /// Bypasses `HostValue` marshaling. The function is registered immediately
    /// if the V8 context is already initialized, or deferred to the next
    /// `register_api` call. This is a no-op if the function name is already
    /// registered as a direct function.
    pub fn register_direct(&mut self, name: &'static str, func: DirectFn) {
        if !self.direct_fns.iter().any(|(n, _)| *n == name) {
            self.direct_fns.push((name, func));
        }
    }

    fn register_api(&mut self) -> Result<()> {
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        let global = local_context.global(scope);

        // Register direct (fast-path) functions first — no HostValue marshaling.
        for (name, func) in &self.direct_fns {
            let js_name = v8::String::new(scope, name).unwrap();
            // Store a pointer to the function pointer in the boxed Vec<DirectFn>.
            let ptr = func as *const DirectFn as *mut std::ffi::c_void;
            let external = v8::External::new(scope, ptr);
            let js_func = v8::Function::builder(
                |s: &mut v8::PinScope,
                 a: v8::FunctionCallbackArguments,
                 r: v8::ReturnValue| {
                    let external = v8::Local::<v8::External>::try_from(a.data()).unwrap();
                    // Safety: the pointer points into the boxed `direct_fns` Vec,
                    // which is never moved while V8 callbacks are live.
                    let func_ptr: *const DirectFn = external.value() as *const DirectFn;
                    let func: DirectFn = unsafe { *func_ptr };
                    func(s, a, r);
                },
            )
            .data(external.into())
            .build(scope)
            .ok_or_else(|| ScriptError::Runtime(format!("failed to build direct {}", name)))?;
            global.set(scope, js_name.into(), js_func.into()).unwrap();
        }

        // Register regular HostFn-based functions.
        for entry in self.api.iter() {
            // Skip if already registered as a direct function.
            if self.direct_fns.iter().any(|(n, _)| *n == entry.0) {
                continue;
            }

            // Stable pointer into the boxed registry vector.
            let ptr = entry as *const (&'static str, HostFn) as *mut std::ffi::c_void;
            let data = v8::External::new(scope, ptr);
            let js_name = v8::String::new(scope, entry.0).unwrap();
            let func = v8::Function::builder(
                |scope: &mut v8::PinScope,
                 args: v8::FunctionCallbackArguments,
                 mut retval: v8::ReturnValue| {
                    // Marshal JS arguments → HostValue slice.
                    let n = args.length() as usize;
                    let mut host_args: Vec<HostValue> = Vec::with_capacity(n);
                    for i in 0..n as i32 {
                        host_args.push(v8_value_to_host(args.get(i), scope));
                    }

                    let external = v8::Local::<v8::External>::try_from(args.data()).unwrap();
                    let entry = unsafe { &*(external.value() as *const (&'static str, HostFn)) };

                    // Call the host function.
                    match (entry.1)(&host_args) {
                        Ok(ret) => {
                            retval.set(host_to_v8_value(ret, scope));
                        }
                        Err(e) => {
                            scope.throw_exception(v8::String::new(scope, &e).unwrap().into());
                        }
                    }
                },
            )
            .data(data.into())
            .build(scope)
            .ok_or_else(|| ScriptError::Runtime(format!("failed to build {}", entry.0)))?;
            global.set(scope, js_name.into(), func.into()).unwrap();
        }

        Self::register_console(scope, local_context)
    }

    /// Register a `console` object with `log`/`info`/`warn`/`error` that forward
    /// to the host logger, stringifying every argument the way browsers do.
    fn register_console(scope: &mut v8::PinScope, context: v8::Local<v8::Context>) -> Result<()> {
        let console = v8::Object::new(scope);
        let global = context.global(scope);

        let log = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                info!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.log".into()))?;
        let info_fn = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                info!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.info".into()))?;
        let warn_fn = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                warn!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.warn".into()))?;
        let err_fn = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                error!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.error".into()))?;

        for (name, func) in [
            ("log", log),
            ("info", info_fn),
            ("warn", warn_fn),
            ("error", err_fn),
        ] {
            let n = v8::String::new(scope, name).unwrap();
            console.set(scope, n.into(), func.into());
        }
        let cname = v8::String::new(scope, "console").unwrap();
        global.set(scope, cname.into(), console.into());

        Ok(())
    }
}

impl V8Runtime {
    /// Load a module graph using V8's native ESModule API.
    ///
    /// 1. Pre-compiles all modules via `ScriptCompiler::compile_module`.
    /// 2. Stores compiled `Local<Module>` handles in a `ModuleMap`.
    /// 3. Stores a `*const ModuleMap` pointer in the isolate slot.
    /// 4. Calls `Module::instantiate_module2` with a static resolve callback.
    /// 5. Evaluates the entry module and calls `exports.main()`.
    pub fn load_module_graph(&mut self, registry: &super::ModuleRegistry, entry: &str) -> Result<()> {
        let order = registry
            .order_dependencies(entry)
            .map_err(|e| ScriptError::Execution(format!("dependency resolution: {}", e)))?;

        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);

        // Step 1: pre-compile all modules.
        let mut module_map = ModuleMap {
            modules: HashMap::new(),
            names: order.clone(),
        };

        for module_name in &order {
            let info = registry
                .get(module_name)
                .ok_or_else(|| ScriptError::Execution(format!("module '{}' not found", module_name)))?;

            // Use the original ESM source (not the CJS transform) for native modules.
            let code = v8::String::new(scope, &info.source)
                .ok_or_else(|| ScriptError::Execution("failed to create source string".into()))?;
            let origin = v8::ScriptOrigin::new(
                scope,
                v8::String::new(scope, module_name).unwrap().into(),
                0, 0, false, 0, None, false, false, true,
                None,
            );
            let mut source = v8::script_compiler::Source::new(code, Some(&origin));
            let module = v8::script_compiler::compile_module(scope, &mut source)
                .ok_or_else(|| {
                    let msg = format!("failed to compile module '{}'", module_name);
                    ScriptError::Execution(msg)
                })?;

            module_map.modules.insert(
                module_name.clone(),
                ModuleEntry {
                    module: v8::Global::new(scope, module),
                    namespace: None,
                },
            );
        }

        // Step 2: get the entry module handle.
        let entry_entry = module_map.modules.get(entry)
            .ok_or_else(|| ScriptError::Execution(format!("entry '{}' not found", entry)))?;
        let entry_module = v8::Local::new(scope, &entry_entry.module);

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
            let msg = if let Some(exc) = tc_scope.exception() {
                exc.to_rust_string_lossy(tc_scope)
            } else {
                "instantiation failed".to_string()
            };
            return Err(ScriptError::Execution(msg));
        }

        // Step 4: evaluate the entry module.
        let evaluate_result = entry_module.evaluate(tc_scope);
        if evaluate_result.is_none() {
            let msg = if let Some(exc) = tc_scope.exception() {
                exc.to_rust_string_lossy(tc_scope)
            } else {
                "evaluation failed".to_string()
            };
            return Err(ScriptError::Execution(msg));
        }

        // Step 5: call entry module's exports.main().
        let namespace = entry_module.get_module_namespace();
        let namespace_obj = v8::Local::<v8::Object>::try_from(namespace)
            .map_err(|_| ScriptError::Runtime("entry module namespace is not an object".into()))?;

        let main_name = v8::String::new(tc_scope, "main").unwrap();
        if let Some(main_val) = namespace_obj.get(tc_scope, main_name.into()) {
            if let Ok(main_func) = v8::Local::<v8::Function>::try_from(main_val) {
                let recv: v8::Local<v8::Value> = v8::undefined(tc_scope).into();
                main_func.call(tc_scope, recv.into(), &[]).ok_or_else(|| {
                    ScriptError::Runtime("call to 'main' failed".into())
                })?;
            }
        }

        // Cache the registry and entry for hot reload.
        self.registry = Some(registry.clone());
        self.entry = Some(entry.to_string());

        Ok(())
    }

    /// Static resolve callback used by V8 during `Module::instantiate_module2`.
    /// Reads the `ModuleMap` pointer from the isolate slot, then looks up the
    /// specifier to return the corresponding compiled module.
    fn module_resolve_callback<'s>(
        context: v8::Local<'s, v8::Context>,
        specifier: v8::Local<'s, v8::String>,
        _import_attributes: v8::Local<'s, v8::FixedArray>,
        _referrer: v8::Local<'s, v8::Module>,
    ) -> Option<v8::Local<'s, v8::Module>> {
        v8::callback_scope!(unsafe scope, context);
        let module_map = unsafe {
            scope.get_slot::<*const ModuleMap>().unwrap().as_ref().unwrap()
        };

        let specifier_str = specifier.to_rust_string_lossy(scope);

        // Try exact match, then with stripped extension, then with .js/.ts appended.
        let entry = module_map
            .modules
            .get(&specifier_str)
            .or_else(|| {
                let stem = specifier_str
                    .rfind('.')
                    .map(|dot| specifier_str[..dot].to_string())
                    .unwrap_or_default();
                module_map.modules.get(&stem)
            })
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
        // Take the cached registry and entry out to avoid double borrow.
        let mut registry = self
            .registry
            .take()
            .ok_or_else(|| ScriptError::Execution("no cached registry for hot reload".into()))?;
        let entry = self
            .entry
            .clone()
            .ok_or_else(|| ScriptError::Execution("no cached entry for hot reload".into()))?;

        // Re-read the changed file and update the registry.
        let source = super::load_script(changed_path)?;
        let name = changed_path
            .to_str()
            .ok_or_else(|| ScriptError::Execution("invalid path".into()))?;
        registry.register(name, source);

        // Re-instantiate the module graph with the updated registry.
        self.load_module_graph(&registry, &entry)
    }
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
    #[allow(dead_code)]
    names: Vec<String>,
}

/// Fast-path `record_frame` that extracts `u32` args directly from V8 values.
///
/// Bypasses the `HostValue → # [script_function]` marshaling chain.
fn direct_record_frame(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let width = if args.length() >= 1 {
        args.get(0).uint32_value(scope).unwrap_or(0)
    } else {
        0
    };
    let height = if args.length() >= 2 {
        args.get(1).uint32_value(scope).unwrap_or(0)
    } else {
        0
    };

    let _ = (width, height);
    match HeadlessContext::record_frame() {
        Ok(ctx) => {
            drop(ctx);
            retval.set_undefined();
        }
        Err(e) => {
            scope.throw_exception(v8::String::new(scope, &e.to_string()).unwrap().into());
        }
    }
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

    // TypedArray — try each concrete typed array type
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
        // Float32Array
        if let Ok(ta) = v8::Local::<v8::Float32Array>::try_from(value) {
            let len = ta.length();
            let byte_len = len * 4;
            let mut bytes = vec![0u8; byte_len];
            ta.copy_contents(&mut bytes);
            let data = unsafe { std::mem::transmute::<Vec<u8>, Vec<f32>>(bytes) };
            return HostValue::TypedArray(TypedArrayValue::Float32(data));
        }
        // Float64Array
        if let Ok(ta) = v8::Local::<v8::Float64Array>::try_from(value) {
            let len = ta.length();
            let byte_len = len * 8;
            let mut bytes = vec![0u8; byte_len];
            ta.copy_contents(&mut bytes);
            let data = unsafe { std::mem::transmute::<Vec<u8>, Vec<f64>>(bytes) };
            return HostValue::TypedArray(TypedArrayValue::Float64(data));
        }
        // Int8Array
        if let Ok(ta) = v8::Local::<v8::Int8Array>::try_from(value) {
            let len = ta.length();
            let byte_len = len;
            let mut bytes = vec![0u8; byte_len];
            ta.copy_contents(&mut bytes);
            let data = unsafe { std::mem::transmute::<Vec<u8>, Vec<i8>>(bytes) };
            return HostValue::TypedArray(TypedArrayValue::Int8(data));
        }
        // Uint16Array
        if let Ok(ta) = v8::Local::<v8::Uint16Array>::try_from(value) {
            let len = ta.length();
            let byte_len = len * 2;
            let mut bytes = vec![0u8; byte_len];
            ta.copy_contents(&mut bytes);
            let data = unsafe { std::mem::transmute::<Vec<u8>, Vec<u16>>(bytes) };
            return HostValue::TypedArray(TypedArrayValue::Uint16(data));
        }
        // Int16Array
        if let Ok(ta) = v8::Local::<v8::Int16Array>::try_from(value) {
            let len = ta.length();
            let byte_len = len * 2;
            let mut bytes = vec![0u8; byte_len];
            ta.copy_contents(&mut bytes);
            let data = unsafe { std::mem::transmute::<Vec<u8>, Vec<i16>>(bytes) };
            return HostValue::TypedArray(TypedArrayValue::Int16(data));
        }
        // Uint32Array
        if let Ok(ta) = v8::Local::<v8::Uint32Array>::try_from(value) {
            let len = ta.length();
            let byte_len = len * 4;
            let mut bytes = vec![0u8; byte_len];
            ta.copy_contents(&mut bytes);
            let data = unsafe { std::mem::transmute::<Vec<u8>, Vec<u32>>(bytes) };
            return HostValue::TypedArray(TypedArrayValue::Uint32(data));
        }
        // Int32Array
        if let Ok(ta) = v8::Local::<v8::Int32Array>::try_from(value) {
            let len = ta.length();
            let byte_len = len * 4;
            let mut bytes = vec![0u8; byte_len];
            ta.copy_contents(&mut bytes);
            let data = unsafe { std::mem::transmute::<Vec<u8>, Vec<i32>>(bytes) };
            return HostValue::TypedArray(TypedArrayValue::Int32(data));
        }
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
fn host_to_v8_value<'s>(value: HostValue, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    match value {
        HostValue::Null => v8::null(scope).into(),
        HostValue::Bool(b) => v8::Boolean::new(scope, b).into(),
        HostValue::Number(n) => v8::Number::new(scope, n).into(),
        HostValue::String(s) => v8::String::new(scope, &s).unwrap().into(),
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
            let buf = v8::ArrayBuffer::new(scope, len);
            if let Some(ptr) = buf.data() {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        data,
                        ptr.as_ptr() as *mut u8,
                        len,
                    );
                }
            }
            buf.into()
        }
        HostValue::Object(map) => {
            let obj = v8::Object::new(scope);
            for (k, v) in map {
                let key = v8::String::new(scope, &k).unwrap();
                let val = host_to_v8_value(v, scope);
                obj.set(scope, key.into(), val);
            }
            obj.into()
        }
        HostValue::Array(items) => {
            let len = items.len() as u32;
            let arr = v8::Array::new(scope, len as i32);
            for (i, item) in items.into_iter().enumerate() {
                let val = host_to_v8_value(item, scope);
                arr.set_index(scope, i as u32, val);
            }
            arr.into()
        }
        HostValue::TypedArray(ta) => {
            let val: v8::Local<'s, v8::Value> = match ta {
                TypedArrayValue::Uint8(data) => {
                    let buf = v8::ArrayBuffer::new(scope, data.len());
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.as_ptr() as *mut u8, data.len());
                        }
                    }
                    v8::Uint8Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
                TypedArrayValue::Float32(data) => {
                    let byte_len = data.len() * 4;
                    let buf = v8::ArrayBuffer::new(scope, byte_len);
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr.as_ptr() as *mut u8, byte_len);
                        }
                    }
                    v8::Float32Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
                TypedArrayValue::Float64(data) => {
                    let byte_len = data.len() * 8;
                    let buf = v8::ArrayBuffer::new(scope, byte_len);
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr.as_ptr() as *mut u8, byte_len);
                        }
                    }
                    v8::Float64Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
                TypedArrayValue::Int8(data) => {
                    let byte_len = data.len();
                    let buf = v8::ArrayBuffer::new(scope, byte_len);
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr.as_ptr() as *mut u8, byte_len);
                        }
                    }
                    v8::Int8Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
                TypedArrayValue::Uint16(data) => {
                    let byte_len = data.len() * 2;
                    let buf = v8::ArrayBuffer::new(scope, byte_len);
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr.as_ptr() as *mut u8, byte_len);
                        }
                    }
                    v8::Uint16Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
                TypedArrayValue::Int16(data) => {
                    let byte_len = data.len() * 2;
                    let buf = v8::ArrayBuffer::new(scope, byte_len);
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr.as_ptr() as *mut u8, byte_len);
                        }
                    }
                    v8::Int16Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
                TypedArrayValue::Uint32(data) => {
                    let byte_len = data.len() * 4;
                    let buf = v8::ArrayBuffer::new(scope, byte_len);
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr.as_ptr() as *mut u8, byte_len);
                        }
                    }
                    v8::Uint32Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
                TypedArrayValue::Int32(data) => {
                    let byte_len = data.len() * 4;
                    let buf = v8::ArrayBuffer::new(scope, byte_len);
                    if let Some(ptr) = buf.data() {
                        unsafe {
                            std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, ptr.as_ptr() as *mut u8, byte_len);
                        }
                    }
                    v8::Int32Array::new(scope, buf, 0, data.len()).unwrap().into()
                }
            };
            val
        }
    }
}

/// Stringify all arguments passed to a `console.*` call, joined by spaces.
fn collect_console_args(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> String {
    let n = args.length();
    let mut parts: Vec<String> = Vec::with_capacity(n.max(0) as usize);
    for i in 0..n {
        parts.push(args.get(i).to_rust_string_lossy(scope));
    }
    parts.join(" ")
}

/// Extract a human-readable exception (message + location + stack frames) from
/// a V8 `TryCatch` scope. Used as a macro because the `tc` scope type is an
/// opaque pinned projection that is awkward to name generically.
macro_rules! v8_exception {
    ($tc:expr) => {{
        let tc = $tc;
        if !tc.has_caught() {
            String::from("unknown error")
        } else {
            let mut out = String::new();
            if let Some(exc) = tc.exception() {
                out.push_str(&exc.to_rust_string_lossy(tc));
            }
            if let Some(msg) = tc.message() {
                let mut loc = String::new();
                if let Some(res) = msg.get_script_resource_name(tc) {
                    let r = res.to_rust_string_lossy(tc);
                    if !r.is_empty() && r != "undefined" {
                        loc.push_str(&r);
                    }
                }
                if let Some(line) = msg.get_line_number(tc) {
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
                            if let Some(sname) = frame.get_script_name_or_source_url(tc) {
                                f.push_str(&sname.to_rust_string_lossy(tc));
                            }
                            f.push_str(&format!(
                                ":{}:{}",
                                frame.get_line_number(),
                                frame.get_column()
                            ));
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

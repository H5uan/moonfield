//! TypeScript/JavaScript scripting runtime for moonfield.
//!
//! The runtime is backend-agnostic through the [`ScriptRuntime`] trait. The
//! default backend is V8 via the `v8` crate; the `rquickjs` backend is
//! available behind the `quickjs-backend` feature.
//!
//! # TypeScript compilation
//!
//! TypeScript is loaded by stripping type annotations with swc at runtime
//! (see [`transpile_typescript`]), on both backends — no `tsc` step is
//! required. Note this is *type stripping only*: type-only syntax vanishes,
//! but TS-only runtime constructs (enums, namespaces, parameter properties)
//! are not supported.
//!
//! A requested `.ts` file is always transpiled from its own source — a
//! pre-compiled `.js` (e.g. `tsc` output in `target/scripts/` or alongside
//! the `.ts`) is never substituted for it, so editing the `.ts` can never
//! (re)load stale build output. Plain `.js` files are still read directly,
//! and the V8 backend remaps their error locations back to the original TS
//! positions via sibling `.js.map` files (see `source_map`).

pub mod api;
pub mod hot_reload;
pub mod module;

#[cfg(feature = "v8-backend")]
pub(crate) mod source_map;

#[cfg(feature = "quickjs-backend")]
pub mod quickjs;

#[cfg(feature = "v8-backend")]
pub mod v8_runtime;

pub use api::HostFn;
pub use api::HostValue;
pub use api::ScriptApi;
pub use api::ScriptFunction;
pub use api::TypedArrayElement;
pub use api::TypedArrayValue;
pub use hot_reload::HotReloadHandler;
pub use hot_reload::HotReloader;
pub use module::ModuleRegistry;

#[cfg(feature = "quickjs-backend")]
pub use quickjs::QuickJsRuntime;

#[cfg(feature = "v8-backend")]
pub use v8_runtime::V8Runtime;

use std::path::{Path, PathBuf};

/// Default maximum JS heap size (both backends).
pub const DEFAULT_MAX_HEAP_BYTES: usize = 128 * 1024 * 1024;

/// Default per-execution timeout for top-level script calls (both backends).
///
/// The watchdog interrupts *JavaScript* execution only: V8's
/// `terminate_execution` takes effect at JS interrupt checks and QuickJS's
/// interrupt handler runs between bytecodes. A script blocked inside a host
/// function's native call (e.g. a Vulkan driver call like `record_frame`)
/// is past the watchdog's reach — it hangs the main thread past any
/// timeout.
pub const DEFAULT_EXECUTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Maximum nesting depth when converting a JS value to a [`HostValue`]
/// (both backends). A cyclic value has unbounded depth — without a cap the
/// conversion recurses until the Rust stack overflows (a process abort that
/// `catch_unwind` cannot stop). Containers deeper than this degrade to a
/// stringified placeholder instead.
pub(crate) const MAX_HOST_VALUE_DEPTH: usize = 64;

/// Errors that can occur in the scripting layer.
#[derive(Debug)]
pub enum ScriptError {
    /// The requested backend is not available.
    BackendNotAvailable(String),
    /// Failed to transpile TypeScript to JavaScript.
    Transpile(String),
    /// Failed to load or execute a script.
    Execution(String),
    /// A runtime error occurred inside the JS engine.
    Runtime(String),
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScriptError::BackendNotAvailable(name) => {
                write!(f, "script backend '{}' is not available", name)
            }
            ScriptError::Transpile(msg) => write!(f, "script transpile failed: {}", msg),
            ScriptError::Execution(msg) => write!(f, "script execution failed: {}", msg),
            ScriptError::Runtime(msg) => write!(f, "script runtime error: {}", msg),
        }
    }
}

impl std::error::Error for ScriptError {}

/// Result type for script operations.
pub type Result<T> = std::result::Result<T, ScriptError>;

/// Extract a human-readable message from a `catch_unwind` panic payload.
///
/// Used by the backends to turn a panicking host function into a JS
/// exception instead of unwinding across the FFI boundary.
#[cfg_attr(
    not(any(feature = "v8-backend", feature = "quickjs-backend")),
    allow(dead_code)
)]
pub(crate) fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Abstraction over a JavaScript engine backend.
///
/// Backends are responsible for creating an execution context, exposing a set
/// of Rust APIs to scripts, loading transpiled JS, and invoking exported
/// functions.
pub trait ScriptRuntime {
    /// Create a new runtime instance with the provided API bindings.
    fn new(api: ScriptApi) -> Result<Self>
    where
        Self: Sized;

    /// Load a JavaScript module from source code.
    fn load(&mut self, name: &str, source: &str) -> Result<()>;

    /// Reload the runtime, clearing existing state and re-registering APIs.
    fn reload(&mut self) -> Result<()>;

    /// Call a top-level function exported by the loaded script.
    fn call(&mut self, function: &str) -> Result<()>;

    /// Call a function with arguments and return its result.
    ///
    /// Default implementation ignores args and delegates to `call`,
    /// returning `HostValue::Null`.
    fn call_with_args(&mut self, function: &str, args: &[HostValue]) -> Result<HostValue> {
        let _ = args;
        self.call(function)?;
        Ok(HostValue::Null)
    }

    /// Call a function exported from the loaded ESModule with arguments.
    ///
    /// Unlike `call_with_args` (which looks up globals), this looks up
    /// functions on the module namespace object.
    ///
    /// Default implementation delegates to `call_with_args`.
    fn call_module_export(&mut self, function: &str, args: &[HostValue]) -> Result<HostValue> {
        self.call_with_args(function, args)
    }

    /// Call a function exported from the loaded ESModule, discarding its
    /// return value.
    ///
    /// Use this for fire-and-forget calls (e.g. the per-frame `on_update`
    /// hook): backends override this to skip marshaling a result the caller
    /// would throw away anyway.
    ///
    /// Default implementation delegates to `call_module_export`.
    fn call_module_export_unit(&mut self, function: &str, args: &[HostValue]) -> Result<()> {
        self.call_module_export(function, args).map(|_| ())
    }

    /// Check whether a callable named `name` is currently available.
    ///
    /// Backends check the entry module's exports first (when a module graph
    /// is loaded) and fall back to globals. Used to invoke optional
    /// script-side lifecycle hooks (`on_update`, `on_shutdown`) without
    /// producing errors when the script does not define them.
    ///
    /// Default implementation returns `false`.
    fn has_function(&mut self, name: &str) -> bool {
        let _ = name;
        false
    }

    /// Load and evaluate an ESModule graph from a resolved registry.
    ///
    /// `registry` must already contain the entry module and all of its
    /// transitive dependencies (see [`ModuleRegistry::resolve_dependencies`]).
    /// After evaluation, the entry module's `main()` export is called if
    /// present, and the registry is cached for hot reload.
    ///
    /// The default implementation reports the backend as not supporting
    /// module graphs.
    fn load_module_graph(
        &mut self,
        registry: std::rc::Rc<ModuleRegistry>,
        entry: &str,
    ) -> Result<()> {
        let _ = (registry, entry);
        Err(ScriptError::BackendNotAvailable(
            "module graph loading".into(),
        ))
    }

    /// Warm up the JIT compiler by running the entry function a few times.
    ///
    /// V8's JIT (Sparkplug/Turbofan) requires multiple executions before
    /// compiling hot paths. **This executes the function, side effects and
    /// all** — only use it when running the entry point several times is
    /// acceptable, or when the function is side-effect free.
    ///
    /// Default implementation is a no-op for engines without JIT (QuickJS).
    fn warmup(&mut self, function: &str) -> Result<()> {
        // JIT compilers need multiple calls to trigger compilation.
        // Default: 3 iterations to hit baseline JIT thresholds.
        for _ in 0..3 {
            self.call(function)?;
        }
        Ok(())
    }

    /// Run incremental garbage collection during idle time.
    ///
    /// Call this once per frame (e.g. from an update system) to let the JS
    /// engine do incremental GC work, avoiding unpredictable full-GC pauses
    /// during script execution.
    ///
    /// Default implementation is a no-op.
    fn gc_step(&mut self) {}
}

/// Load a script file, resolving TypeScript to JavaScript.
///
/// Loading strategy:
/// 1. If the path ends in `.ts`, read it and strip the type annotations
///    with swc (see [`transpile_typescript`]). The `.ts` source is the
///    single source of truth — a pre-compiled `.js` (e.g. stale `tsc`
///    output in `target/scripts/` or alongside the file) is never
///    substituted for it.
/// 2. Otherwise (`.js` etc.), read the file directly.
pub fn load_script<P: AsRef<Path>>(path: P) -> Result<String> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path)
        .map_err(|e| ScriptError::Execution(format!("failed to read script: {}", e)))?;
    if path.extension().and_then(|e| e.to_str()) == Some("ts") {
        transpile_typescript(&source)
    } else {
        Ok(source)
    }
}

/// Map a changed file to the source of truth a hot reload should read.
///
/// `.ts` is the single source of truth: when a changed `.js` has a `.ts`
/// sibling, the `.ts` shadows it (the `.js` is stale build output), so the
/// event reloads the `.ts` instead — editing the `.js` can never resurrect
/// stale code. A `.js` without a `.ts` sibling, and any other file,
/// reloads from its own path.
#[cfg_attr(
    not(any(feature = "v8-backend", feature = "quickjs-backend")),
    allow(dead_code)
)]
pub(crate) fn reload_source_path(path: &Path) -> PathBuf {
    if path.extension().and_then(|e| e.to_str()) == Some("js") {
        let ts_sibling = path.with_extension("ts");
        if ts_sibling.is_file() {
            return ts_sibling;
        }
    }
    path.to_path_buf()
}

/// Transpile TypeScript source to JavaScript by stripping type annotations.
///
/// Uses swc's TypeScript strip transform to remove type annotations.
/// Type-only syntax is erased; TS-only runtime constructs (enums,
/// namespaces, parameter properties) are not supported.
pub fn transpile_typescript(source: &str) -> Result<String> {
    use swc_common::{sync::Lrc, FileName, Globals, Mark, SourceMap, GLOBALS};
    use swc_ecma_ast::{EsVersion, Module, Pass, Program};
    use swc_ecma_codegen::{text_writer::JsWriter, Config, Emitter};
    use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax, TsSyntax};
    use swc_ecma_transforms_typescript::strip;

    GLOBALS.set(&Globals::default(), || {
        let source = source.to_string();
        let cm: Lrc<SourceMap> = Default::default();
        let fm = cm.new_source_file(Lrc::new(FileName::Custom("script.ts".into())), source);

        let lexer = Lexer::new(
            Syntax::Typescript(TsSyntax::default()),
            EsVersion::Es2020,
            StringInput::from(&*fm),
            None,
        );
        let mut parser = Parser::new_from(lexer);

        let module: Module = parser
            .parse_module()
            .map_err(|e| ScriptError::Transpile(format!("parse error: {:?}", e)))?;
        let recoverable = parser.take_errors();
        if !recoverable.is_empty() {
            return Err(ScriptError::Transpile(format!(
                "parse error: {:?}",
                recoverable
            )));
        }

        let mut program = Program::Module(module);
        let mut pass = strip(Mark::new(), Mark::new());
        pass.process(&mut program);
        let module = match program {
            Program::Module(m) => m,
            _ => return Err(ScriptError::Transpile("expected module".into())),
        };

        let mut buf = Vec::new();
        {
            let mut emitter = Emitter {
                cfg: Config::default(),
                cm: cm.clone(),
                comments: None,
                wr: JsWriter::new(cm.clone(), "\n", &mut buf, None),
            };
            emitter
                .emit_module(&module)
                .map_err(|e| ScriptError::Transpile(format!("codegen error: {:?}", e)))?;
        }
        String::from_utf8(buf)
            .map_err(|e| ScriptError::Transpile(format!("non-utf8 output: {}", e)))
    })
}

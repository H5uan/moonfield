//! TypeScript/JavaScript scripting runtime for moonfield.
//!
//! The runtime is backend-agnostic through the [`ScriptRuntime`] trait. The
//! default backend is V8 via the `v8` crate; the `rquickjs` backend is
//! available behind the `quickjs-backend` feature.
//!
//! # TypeScript compilation
//!
//! TypeScript is compiled to JavaScript at build time via `tsc` (see
//! `scripts/tsconfig.json`). The runtime loads pre-compiled `.js` files from
//! `target/scripts/` or alongside the `.ts` source.
//!
//! For the V8 backend, V8's native `--strip-types` flag also allows loading
//! `.ts` files directly at runtime — no preprocessing needed. QuickJS has no
//! native TS support, so the `quickjs-backend` feature optionally enables
//! swc-based transpilation as a fallback.

pub mod api;
pub mod hot_reload;
pub mod module;

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
    fn call_module_export(
        &mut self,
        function: &str,
        args: &[HostValue],
    ) -> Result<HostValue> {
        self.call_with_args(function, args)
    }

    /// Warm up the JIT compiler by running the entry function a few times.
    ///
    /// V8's JIT (Sparkplug/Turbofan) requires multiple executions before
    /// compiling hot paths. Calling this after loading the script ensures
    /// the frame loop runs compiled code from the first frame.
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

/// Load a script file, resolving TypeScript to pre-compiled JavaScript.
///
/// Loading strategy:
/// 1. If the path ends in `.js`, read it directly.
/// 2. If the path ends in `.ts`:
///    a. First look for a `.js` file at `target/scripts/<filename>.js`
///       (build-time tsc output).
///    b. If not found, look for a `.js` file alongside the `.ts` file.
///    c. If neither exists and the `quickjs-backend` feature is active,
///       fall back to swc-based transpilation. Otherwise, return an error
///       (V8 requires pre-compiled JS, or the `.ts` file must be pre-compiled
///       via `tsc`; see `scripts/tsconfig.json`).
pub fn load_script<P: AsRef<Path>>(path: P) -> Result<String> {
    let path = path.as_ref();

    // For .js files, load directly.
    if path.extension().and_then(|e| e.to_str()) != Some("ts") {
        return std::fs::read_to_string(path)
            .map_err(|e| ScriptError::Execution(format!("failed to read script: {}", e)));
    }

    // Try pre-compiled JS from tsc build output (target/scripts/).
    let ts_filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let js_filename = ts_filename.replace(".ts", ".js");

    // Try target/scripts/ relative to project root.
    let js_paths = [
        PathBuf::from("target/scripts").join(&js_filename),
        path.with_extension("js"),
    ];

    for js_path in &js_paths {
        if js_path.exists() {
            return std::fs::read_to_string(js_path)
                .map_err(|e| ScriptError::Execution(format!("failed to read script: {}", e)));
        }
    }

    // No pre-compiled JS found. QuickJS backend can fall back to swc transpilation;
    // V8 backend requires pre-compiled JS.
    #[cfg(feature = "quickjs-backend")]
    {
        let source = std::fs::read_to_string(path)
            .map_err(|e| ScriptError::Execution(format!("failed to read script: {}", e)))?;
        return transpile_typescript(&source);
    }

    #[cfg(not(feature = "quickjs-backend"))]
    Err(ScriptError::Execution(format!(
        "no pre-compiled JavaScript found for '{}'. \
         TypeScript must be compiled via `tsc` before running. \
         See `scripts/tsconfig.json`.",
        path.display()
    )))
}

/// Transpile TypeScript source to JavaScript by stripping type annotations.
///
/// Uses swc's TypeScript strip transform to remove type annotations.
/// Only available when the `quickjs-backend` feature is active.
#[cfg(feature = "quickjs-backend")]
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

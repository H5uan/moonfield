//! TypeScript/JavaScript scripting runtime for moonfield.
//!
//! The runtime is backend-agnostic through the [`ScriptRuntime`] trait. The
//! default backend is V8 via the `v8` crate; the `rquickjs` backend is
//! available behind the `quickjs-backend` feature.

pub mod api;
pub mod hot_reload;

#[cfg(feature = "quickjs-backend")]
pub mod quickjs;

#[cfg(feature = "v8-backend")]
pub mod v8_runtime;

pub use api::HostFn;
pub use api::ScriptApi;
pub use hot_reload::HotReloader;

#[cfg(feature = "quickjs-backend")]
pub use quickjs::QuickJsRuntime;

#[cfg(feature = "v8-backend")]
pub use v8_runtime::V8Runtime;

use std::path::Path;

use swc_common::{sync::Lrc, FileName, Globals, Mark, SourceMap, GLOBALS};
use swc_ecma_ast::{EsVersion, Module, Pass, Program};
use swc_ecma_codegen::{text_writer::JsWriter, Config, Emitter};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax, TsSyntax};
use swc_ecma_transforms_typescript::strip;

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
}

/// Convenience helper: transpile a TypeScript file to JavaScript if needed and
/// return the JS source.
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

/// Transpile TypeScript source to JavaScript by stripping type annotations.
///
/// Backed by `swc` (real parser + type-strip transform + codegen), so it
/// correctly handles generics, type assertions, interfaces, enums, etc. — not
/// just line-level stripping. The `GLOBALS` thread-local is required by swc's
/// hygiene/marks machinery, so transpilation runs inside `GLOBALS.set`.
pub fn transpile_typescript(source: &str) -> Result<String> {
    GLOBALS.set(&Globals::default(), || transpile_inner(source))
}

fn transpile_inner(source: &str) -> Result<String> {
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

    // `strip` returns an `impl Pass`; `Pass::process` mutates a `Program`.
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
    String::from_utf8(buf).map_err(|e| ScriptError::Transpile(format!("non-utf8 output: {}", e)))
}

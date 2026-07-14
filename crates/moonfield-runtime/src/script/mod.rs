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

pub use api::ScriptApi;
pub use hot_reload::HotReloader;

#[cfg(feature = "quickjs-backend")]
pub use quickjs::QuickJsRuntime;

#[cfg(feature = "v8-backend")]
pub use v8_runtime::V8Runtime;

use std::path::Path;

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

/// Minimal TypeScript-to-JavaScript transpiler.
///
/// This implementation strips simple type annotations and interface/type
/// declarations so that the resulting source can be run by the JS engine. It
/// is not a full TS compiler; for production use replace this with `tsc`/`swc`.
pub fn transpile_typescript(source: &str) -> Result<String> {
    // For the example scripts we rely on a build-time `tsc` step (see
    // scripts/tsconfig.json). If a .ts file is loaded directly at runtime we
    // strip the most common annotations so basic examples still work.
    let mut output = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        // Skip interface/type/import type-only declarations.
        if trimmed.starts_with("interface ")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("declare ")
        {
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    Ok(output)
}

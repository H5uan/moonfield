//! Rust APIs exposed to scripts.

use moonfield_lunaris::HeadlessContext;

/// A value that can be passed between scripts and host functions.
///
/// Backends marshal between their native JS types and `HostValue` so that
/// host functions work with a uniform, engine-agnostic type system.
#[derive(Debug, Clone)]
pub enum HostValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    ArrayBuffer(Vec<u8>),
}

impl HostValue {
    /// Try to extract an `f64` from this value.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            HostValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Try to extract a `bool` from this value.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            HostValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to extract a `&str` from this value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            HostValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Try to extract a `u32` from this value.
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            HostValue::Number(n) => {
                if *n >= 0.0 && *n <= u32::MAX as f64 {
                    Some(*n as u32)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl From<f64> for HostValue {
    fn from(v: f64) -> Self {
        HostValue::Number(v)
    }
}

impl From<i32> for HostValue {
    fn from(v: i32) -> Self {
        HostValue::Number(v as f64)
    }
}

impl From<bool> for HostValue {
    fn from(v: bool) -> Self {
        HostValue::Bool(v)
    }
}

impl From<String> for HostValue {
    fn from(v: String) -> Self {
        HostValue::String(v)
    }
}

impl From<&str> for HostValue {
    fn from(v: &str) -> Self {
        HostValue::String(v.to_string())
    }
}

/// A host function exposed to scripts.
///
/// Receives a slice of arguments and returns a value (or an error string).
/// Backends handle the JS ↔ HostValue marshaling automatically.
pub type HostFn = fn(&[HostValue]) -> Result<HostValue, String>;

/// Registry of host functions made available to scripts.
#[derive(Clone)]
pub struct ScriptApi {
    functions: Vec<(&'static str, HostFn)>,
}

impl ScriptApi {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
        }
    }

    /// Register a host function under `name` (the global identifier scripts
    /// call). Chainable.
    pub fn register(&mut self, name: &'static str, f: HostFn) -> &mut Self {
        self.functions.push((name, f));
        self
    }

    /// Iterate over the registered `(name, function)` entries.
    pub fn iter(&self) -> std::slice::Iter<'_, (&'static str, HostFn)> {
        self.functions.iter()
    }
}

impl Default for ScriptApi {
    fn default() -> Self {
        let mut api = Self::new();
        api.register("record_frame", default_record_frame);
        api
    }
}

/// Default `record_frame` host function.
///
/// Accepts optional `(width: u32, height: u32)` arguments.
/// Defaults to the headless context's default resolution.
fn default_record_frame(args: &[HostValue]) -> Result<HostValue, String> {
    let _width = args.first().and_then(|v| v.as_u32());
    let _height = args.get(1).and_then(|v| v.as_u32());
    let ctx = HeadlessContext::record_frame().map_err(|e| e.to_string())?;
    drop(ctx);
    Ok(HostValue::Null)
}

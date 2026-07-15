//! Rust APIs exposed to scripts.

use std::collections::HashMap;

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
    /// A JavaScript plain object (string-keyed map).
    Object(HashMap<String, HostValue>),
    /// A JavaScript array.
    Array(Vec<HostValue>),
    /// A typed array (e.g. Float32Array, Uint8Array).
    TypedArray(TypedArrayValue),
}

/// Represents a JavaScript typed array with its element type and data.
#[derive(Debug, Clone)]
pub enum TypedArrayValue {
    Uint8(Vec<u8>),
    Int8(Vec<i8>),
    Uint16(Vec<u16>),
    Int16(Vec<i16>),
    Uint32(Vec<u32>),
    Int32(Vec<i32>),
    Float32(Vec<f32>),
    Float64(Vec<f64>),
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

    /// Try to extract an `&HashMap<String, HostValue>` from this value.
    pub fn as_object(&self) -> Option<&HashMap<String, HostValue>> {
        match self {
            HostValue::Object(m) => Some(m),
            _ => None,
        }
    }

    /// Try to extract a `&[HostValue]` from this value.
    pub fn as_array(&self) -> Option<&[HostValue]> {
        match self {
            HostValue::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// Try to extract a `&TypedArrayValue` from this value.
    pub fn as_typed_array(&self) -> Option<&TypedArrayValue> {
        match self {
            HostValue::TypedArray(t) => Some(t),
            _ => None,
        }
    }

    /// Try to extract `&[u8]` from ArrayBuffer or Uint8 typed array.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            HostValue::ArrayBuffer(buf) => Some(buf.as_slice()),
            HostValue::TypedArray(TypedArrayValue::Uint8(buf)) => Some(buf.as_slice()),
            _ => None,
        }
    }

    /// Try to extract `&[f32]` from Float32 typed array.
    pub fn as_f32_slice(&self) -> Option<&[f32]> {
        match self {
            HostValue::TypedArray(TypedArrayValue::Float32(buf)) => Some(buf.as_slice()),
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

impl From<HashMap<String, HostValue>> for HostValue {
    fn from(v: HashMap<String, HostValue>) -> Self {
        HostValue::Object(v)
    }
}

impl From<Vec<HostValue>> for HostValue {
    fn from(v: Vec<HostValue>) -> Self {
        HostValue::Array(v)
    }
}

impl From<Vec<u8>> for HostValue {
    fn from(v: Vec<u8>) -> Self {
        HostValue::ArrayBuffer(v)
    }
}

impl From<Vec<f32>> for HostValue {
    fn from(v: Vec<f32>) -> Self {
        HostValue::TypedArray(TypedArrayValue::Float32(v))
    }
}

impl From<Vec<f64>> for HostValue {
    fn from(v: Vec<f64>) -> Self {
        HostValue::TypedArray(TypedArrayValue::Float64(v))
    }
}

/// A host function exposed to scripts.
///
/// Receives a slice of arguments and returns a value (or an error string).
/// Backends handle the JS ↔ HostValue marshaling automatically.
pub type HostFn = fn(&[HostValue]) -> Result<HostValue, String>;

/// Trait for static type-safe host functions.
///
/// Implemented automatically by the `#[script_function]` proc-macro.
/// Provides a typed bridge between Rust functions and the dynamic
/// [`HostValue`]-based calling convention.
pub trait ScriptFunction {
    /// The name exposed to scripts (e.g. `"record_frame"`).
    const NAME: &'static str;

    /// Call the function with marshaled arguments.
    fn call(args: &[HostValue]) -> Result<HostValue, String>;
}

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

    /// Register a type-safe function annotated with `#[script_function]`.
    ///
    /// Uses the [`ScriptFunction`] trait to extract the name and call logic.
    pub fn register_fn<F: ScriptFunction>(&mut self) -> &mut Self {
        self.functions.push((F::NAME, F::call));
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
        api.register_fn::<record_frame_Fn>();
        api
    }
}

/// Default `record_frame` host function.
///
/// Accepts optional `(width: u32, height: u32)` arguments.
/// Defaults to the headless context's default resolution.
#[moonfield_script_macros::script_function]
fn record_frame(width: u32, height: u32) -> Result<(), String> {
    let _ = (width, height);
    let ctx = HeadlessContext::record_frame().map_err(|e| e.to_string())?;
    drop(ctx);
    Ok(())
}

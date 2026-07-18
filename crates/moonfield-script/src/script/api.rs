//! Rust APIs exposed to scripts.

use std::collections::HashMap;
use std::sync::Arc;

/// A value that can be passed between scripts and host functions.
///
/// Backends marshal between their native JS types and `HostValue` so that
/// host functions work with a uniform, engine-agnostic type system.
///
/// This type is deliberately not `Clone`: the zero-copy view variants
/// (`BytesView`, `TypedArrayView`) hold raw pointers into the JS engine's
/// backing store that dangle as soon as the host call returns, and cloning
/// would silently duplicate those pointers.
#[derive(Debug)]
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
    /// Zero-copy view into the JS engine's backing store.
    /// Only valid during the host function call. Must not be stored beyond the callback.
    BytesView {
        data: *const u8,
        len: usize,
    },
    /// Zero-copy view into a typed array's backing store.
    /// `element` records the original JS typed array element type so
    /// callers can safely interpret the data without re-checking alignment.
    /// Only valid during the host function call. Must not be stored.
    TypedArrayView {
        data: *const u8,
        len: usize, // in bytes
        element: TypedArrayElement,
    },
}

/// Element type of a JavaScript typed array, for use with [`HostValue::TypedArrayView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedArrayElement {
    Uint8,
    Int8,
    Uint16,
    Int16,
    Uint32,
    Int32,
    Float32,
    Float64,
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

    /// Try to extract `&[u8]` from ArrayBuffer, Uint8 typed array, BytesView, or TypedArrayView.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            HostValue::ArrayBuffer(buf) => Some(buf.as_slice()),
            HostValue::TypedArray(TypedArrayValue::Uint8(buf)) => Some(buf.as_slice()),
            HostValue::BytesView { data, len } => {
                Some(unsafe { std::slice::from_raw_parts(*data, *len) })
            }
            HostValue::TypedArrayView { data, len, .. } => {
                Some(unsafe { std::slice::from_raw_parts(*data, *len) })
            }
            _ => None,
        }
    }

    /// Try to extract `&[u8]` from BytesView (zero-copy view into JS engine's backing store).
    pub fn as_bytes_view(&self) -> Option<&[u8]> {
        match self {
            HostValue::BytesView { data, len } => {
                Some(unsafe { std::slice::from_raw_parts(*data, *len) })
            }
            _ => None,
        }
    }

    /// Try to extract `&[f32]` from Float32 typed array or TypedArrayView (zero-copy).
    pub fn as_f32_slice(&self) -> Option<&[f32]> {
        match self {
            HostValue::TypedArray(TypedArrayValue::Float32(buf)) => Some(buf.as_slice()),
            HostValue::TypedArrayView {
                data,
                len,
                element: TypedArrayElement::Float32,
            } => {
                let n = *len / std::mem::size_of::<f32>();
                if n > 0 {
                    Some(unsafe { std::slice::from_raw_parts(*data as *const f32, n) })
                } else {
                    Some(&[])
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
///
/// Uses `Arc<dyn Fn>` instead of `fn` pointer so that host functions can
/// capture state via closures. Register closures with
/// [`ScriptApi::register_closure`].
///
/// `Send + Sync` is required so that a [`ScriptApi`] can be carried inside
/// plugins (which must be `Send + Sync`) and shared across engine threads.
pub type HostFn = Arc<dyn Fn(&[HostValue]) -> Result<HostValue, String> + Send + Sync>;

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

    /// TypeScript declaration for this function, e.g.
    /// `"declare function record_frame(width: number, height: number): void"`.
    /// Used by `ScriptApi::generate_dts` to emit a `.d.ts` file for IDE support.
    fn ts_signature() -> &'static str {
        ""
    }
}

/// Registry of host functions made available to scripts.
#[derive(Clone)]
pub struct ScriptApi {
    functions: Vec<(&'static str, HostFn)>,
    /// TypeScript declarations collected during `register_fn` and `declare`.
    ts_declarations: Vec<&'static str>,
}

impl ScriptApi {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            ts_declarations: Vec::new(),
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
    pub fn register_fn<F: ScriptFunction + 'static>(&mut self) -> &mut Self {
        let f: HostFn = Arc::new(F::call);
        self.functions.push((F::NAME, f));
        let sig = F::ts_signature();
        if !sig.is_empty() {
            self.ts_declarations.push(sig);
        }
        self
    }

    /// Register a closure as a host function, allowing captured state.
    ///
    /// This is the primary entry point for stateful host functions that
    /// need to access Rust-side resources (e.g. render context, asset store).
    /// The closure must be `Send + Sync`; use thread-safe primitives
    /// (`Arc<Mutex<_>>`, channels) for captured state.
    pub fn register_closure<F>(&mut self, name: &'static str, f: F) -> &mut Self
    where
        F: Fn(&[HostValue]) -> Result<HostValue, String> + Send + Sync + 'static,
    {
        self.functions.push((name, Arc::new(f)));
        self
    }

    /// Manually declare a TypeScript signature for a `register_closure` function.
    /// Use this so that `generate_dts` includes closures that can't be auto-generated.
    pub fn declare(&mut self, declaration: &'static str) -> &mut Self {
        self.ts_declarations.push(declaration);
        self
    }

    /// Generate a TypeScript declaration file (`.d.ts`) for all registered
    /// host functions. Write the output to `scripts/moonfield.d.ts` for IDE
    /// autocomplete support.
    pub fn generate_dts(&self) -> String {
        let mut s = String::from(
            "/// <reference no-default-lib=\"true\"/>\n\
             // Auto-generated by moonfield ScriptApi::generate_dts() -- do not edit\n\n",
        );
        for decl in &self.ts_declarations {
            s.push_str(decl);
            s.push('\n');
        }
        s
    }

    /// Iterate over the registered `(name, function)` entries.
    pub fn iter(&self) -> std::slice::Iter<'_, (&'static str, HostFn)> {
        self.functions.iter()
    }
}

impl Default for ScriptApi {
    /// An empty registry. Host functions are provided by the embedding
    /// application (e.g. the `moonfield` binary registers `record_frame`),
    /// keeping this crate free of engine-layer dependencies.
    fn default() -> Self {
        Self::new()
    }
}

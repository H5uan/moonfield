//! Rust APIs exposed to scripts.

use moonfield_lunaris::HeadlessContext;

/// A host function exposed to scripts.
///
/// Statelesness is intentional: V8 callbacks must be zero-sized, so each
/// backend stores a pointer to the [`ScriptApi`] registry entry instead of
/// boxing a closure. Host functions that need engine state access it through
/// the same singletons the default `record_frame` uses.
pub type HostFn = fn() -> Result<(), String>;

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

fn default_record_frame() -> Result<(), String> {
    let ctx = HeadlessContext::record_frame().map_err(|e| e.to_string())?;
    drop(ctx);
    Ok(())
}

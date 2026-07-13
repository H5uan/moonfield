//! Rust APIs exposed to scripts.

use moonfield_lunaris::HeadlessContext;

/// Collection of host functions made available to scripts.
#[derive(Clone)]
pub struct ScriptApi {
    /// Called by scripts to record one headless frame.
    pub record_frame: fn() -> Result<(), String>,
}

impl Default for ScriptApi {
    fn default() -> Self {
        Self {
            record_frame: default_record_frame,
        }
    }
}

fn default_record_frame() -> Result<(), String> {
    let ctx = HeadlessContext::record_frame().map_err(|e| e.to_string())?;
    drop(ctx);
    Ok(())
}

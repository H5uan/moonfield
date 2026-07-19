//! Script-facing time API.
//!
//! Provides wall-clock and engine-startup timing, plus a frame counter,
//! without depending on engine-layer resources. The composition root creates
//! a shared [`ScriptTimeState`] handle, registers the `time_*` / `frame_count`
//! host functions with it, and mirrors the state into the script plugin each
//! frame.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::script::{HostValue, ScriptApi};

/// Time state shared between the script plugin's update system (writer) and
/// the `time_*` host functions (readers).
#[derive(Debug)]
pub struct ScriptTimeState {
    startup: Instant,
    frame_count: u64,
}

impl ScriptTimeState {
    /// Create a new time state. The startup instant should be overwritten by
    /// the plugin once the runtime is created (see [`set_startup`]).
    pub fn new() -> Self {
        Self {
            startup: Instant::now(),
            frame_count: 0,
        }
    }

    /// Set the wall-clock instant used for [`time_since_startup`].
    pub fn set_startup(&mut self, startup: Instant) {
        self.startup = startup;
    }

    /// Advance the frame counter by one.
    pub fn increment_frame(&mut self) {
        self.frame_count += 1;
    }

    /// Current wall-clock time in seconds since the Unix epoch.
    pub fn now_secs(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// Seconds elapsed since the script runtime started.
    pub fn since_startup_secs(&self) -> f64 {
        Instant::now().duration_since(self.startup).as_secs_f64()
    }

    /// Number of frames processed since startup.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

impl Default for ScriptTimeState {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared handle to a [`ScriptTimeState`].
pub type SharedTimeState = Arc<Mutex<ScriptTimeState>>;

/// Create the shared time-state handle.
pub fn new_shared_time() -> SharedTimeState {
    Arc::new(Mutex::new(ScriptTimeState::new()))
}

/// Lock the shared time state, tolerating a poisoned mutex.
fn lock(time: &SharedTimeState) -> MutexGuard<'_, ScriptTimeState> {
    time.lock().unwrap_or_else(|e| e.into_inner())
}

/// Register the built-in `time_*` and `frame_count` host functions.
///
/// These are registered here (not in the composition root) because they only
/// read [`ScriptTimeState`] — no engine-layer dependencies.
pub fn register_time_api(api: &mut ScriptApi, time: &SharedTimeState) {
    {
        let handle = Arc::clone(time);
        api.register_closure("time_now", move |_args| {
            Ok(HostValue::Number(lock(&handle).now_secs()))
        });
        api.declare("declare function time_now(): number;");
    }
    {
        let handle = Arc::clone(time);
        api.register_closure("time_since_startup", move |_args| {
            Ok(HostValue::Number(lock(&handle).since_startup_secs()))
        });
        api.declare("declare function time_since_startup(): number;");
    }
    {
        let handle = Arc::clone(time);
        api.register_closure("frame_count", move |_args| {
            Ok(HostValue::Number(lock(&handle).frame_count() as f64))
        });
        api.declare("declare function frame_count(): number;");
    }
}

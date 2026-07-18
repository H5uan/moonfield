//! Base utilities: initialization and shutdown lifecycle.

use std::sync::atomic::{AtomicBool, Ordering};

static LOGGING_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the engine subsystems.
pub fn initialize() {
    let _ = LOGGING_INITIALIZED.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst);
}

/// Shut down the engine subsystems.
pub fn shutdown() {
    let _ = LOGGING_INITIALIZED.compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst);
}

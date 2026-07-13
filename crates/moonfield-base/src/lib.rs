//! Base utilities: logging, severity levels, and macros.

use std::sync::atomic::{AtomicBool, Ordering};

static LOGGING_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialize the logging system.
pub fn initialize() {
    if LOGGING_INITIALIZED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        println!("[moonfield] logging initialized");
    }
}

/// Shut down the logging system.
pub fn shutdown() {
    if LOGGING_INITIALIZED
        .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        println!("[moonfield] logging shutdown");
    }
}

/// Log an informational message.
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        println!("[INFO]  {}", format!($($arg)*))
    };
}

/// Log a warning message.
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        eprintln!("[WARN]  {}", format!($($arg)*))
    };
}

/// Log an error message.
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        eprintln!("[ERROR] {}", format!($($arg)*))
    };
}

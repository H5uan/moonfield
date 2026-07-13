//! Runtime application plugin.
//!
//! Provides a `RuntimePlugin` that registers the core runtime services and
//! lifecycle systems with the application.

use moonfield_base::info;
use moonfield_core::{App, Plugin, Resources};

/// Runtime plugin.
pub struct RuntimePlugin;

impl Plugin for RuntimePlugin {
    fn name(&self) -> &str {
        "Runtime"
    }

    fn build(&self, app: &mut App) {
        app.add_startup_system(|_res: &mut Resources| {
            info!("Runtime startup system");
        });
        app.add_shutdown_system(|_res: &mut Resources| {
            info!("Runtime shutdown system");
        });
    }
}

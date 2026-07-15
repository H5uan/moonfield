//! File watcher based hot-reload for scripts.

use super::{Result, ScriptError};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver};

/// A handler invoked by [`HotReloader`] when a file change is detected.
///
/// Implementations should update the module registry with the new source and
/// re-instantiate the module graph.
pub trait HotReloadHandler {
    /// Called when a `.ts` or `.js` file changes.
    /// `changed_path` is the absolute path of the file that changed.
    fn on_file_changed(&mut self, changed_path: &Path) -> Result<()>;
}

/// Watches a directory recursively and notifies a [`HotReloadHandler`] when
/// `.ts`/`.js` files change.
///
/// Unlike the old full-`reload()` approach, this only triggers when a script
/// file changes, leaving V8 context and host API bindings intact.
pub struct HotReloader {
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<Event>>,
}

impl HotReloader {
    /// Start watching `dir` recursively for changes.
    pub fn new<D: AsRef<Path>>(dir: D) -> Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .map_err(|e| ScriptError::Execution(format!("watcher: {}", e)))?;

        watcher
            .watch(dir.as_ref(), RecursiveMode::Recursive)
            .map_err(|e| ScriptError::Execution(format!("watch dir: {}", e)))?;

        Ok(Self { watcher, rx })
    }

    /// Poll for file system events and forward to the handler.
    ///
    /// This is non-blocking; call it each frame in the engine update loop.
    pub fn poll<H: HotReloadHandler>(&mut self, handler: &mut H) -> Result<()> {
        while let Ok(event) = self.rx.try_recv() {
            let event = event.map_err(|e| ScriptError::Execution(format!("watch event: {}", e)))?;
            if event.kind.is_modify() || event.kind.is_create() {
                for path in &event.paths {
                    let is_script = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e == "ts" || e == "js")
                        .unwrap_or(false);
                    if is_script {
                        handler.on_file_changed(path)?;
                    }
                }
            }
        }
        Ok(())
    }
}

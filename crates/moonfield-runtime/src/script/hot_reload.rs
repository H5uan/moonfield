//! File watcher based hot-reload for scripts.

use super::{Result, ScriptError, ScriptRuntime};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

/// Watches a directory and reloads the runtime when `.ts`/`.js` files change.
pub struct HotReloader {
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<Event>>,
    script_path: PathBuf,
}

impl HotReloader {
    /// Start watching `dir` for changes. `script_path` is the entry script that
    /// will be reloaded on any relevant change.
    pub fn new<P: AsRef<Path>, D: AsRef<Path>>(dir: D, script_path: P) -> Result<Self> {
        let script_path = script_path.as_ref().to_path_buf();
        let (tx, rx) = channel();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .map_err(|e| ScriptError::Execution(format!("watcher: {}", e)))?;

        watcher
            .watch(dir.as_ref(), RecursiveMode::NonRecursive)
            .map_err(|e| ScriptError::Execution(format!("watch dir: {}", e)))?;

        Ok(Self {
            watcher,
            rx,
            script_path,
        })
    }

    /// Poll for file system events and reload the runtime when a script changes.
    ///
    /// This is non-blocking; call it each frame in the engine update loop.
    pub fn poll<R: ScriptRuntime>(&mut self, runtime: &mut R) -> Result<()> {
        while let Ok(event) = self.rx.try_recv() {
            let event = event.map_err(|e| ScriptError::Execution(format!("watch event: {}", e)))?;
            if event.kind.is_modify() || event.kind.is_create() {
                let changed = event.paths.iter().any(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e == "ts" || e == "js")
                        .unwrap_or(false)
                });
                if changed {
                    let source = super::load_script(&self.script_path)?;
                    runtime.reload()?;
                    runtime.load(
                        self.script_path.to_string_lossy().as_ref(),
                        &source,
                    )?;
                }
            }
        }
        Ok(())
    }
}
